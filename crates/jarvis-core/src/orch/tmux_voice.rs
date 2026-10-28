// tmux à la voix (orchestrator/tmux-voice.js) : phrases reconnues localement (sans IA) et relais
// « tout ce que je dis part dans tmux ». Toute action passe par local_tool() : même sécurité, même journal.
//   « lance Claude Code dans tmux [dans le projet stats-ia] », « ouvre tmux », « parle à Claude »,
//   « Jarvis, reprends la main », « dis à Claude de … », « lance npm run dev dans tmux »,
//   « lis l'écran de Claude », « fenêtres tmux », « fenêtre 2 de tmux », « divise tmux à droite »,
//   « nouvelle fenêtre tmux », « ferme la fenêtre claude de tmux », « ferme tmux ».
// Sécurité : écrire à un agent est libre ; taper dans un shell exige le code ; fermer demande un « oui ».

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::rc::Rc;

use serde_json::{json, Value};

use super::intent::Intent;
use super::projects::fold_name;
use super::tmux::{find_agent_process, is_agent, is_claude, process_alive, process_table, Tmux, TmuxError};
use super::{OrchRef, Up};
use crate::js::{self, J};

const RELAY_IDLE_MS: i64 = 30 * 60 * 1000;

/// Clé de comparaison d'un nom de projet / fenêtre / session (« jarvis-v2 » → « jarvisv2 »).
fn name_key(x: &str) -> String {
    crate::re!(r"[^a-z0-9]").replace_all(&fold_name(x), "").into_owned()
}

/// Repli comme le routeur, puis variantes entendues de « tmux » et de « Claude Code » ramenées à une forme.
pub fn fold(text: &str) -> String {
    let t = js::fold_lower(text);
    let t = crate::re!(r#"['’\-?!,;:.…«»"]"#).replace_all(&t, " ");
    let t = crate::re!(r"\s+").replace_all(&t, " ").trim().to_string();
    let t = crate::re!(r"\b(?:t mux|ti mux|timux|temux|tee mux|team mux|tea mux)\b").replace_all(&t, "tmux");
    let t = crate::re!(r"\b(?:cloud|clode|claude) (?:code|kod|codes)\b").replace_all(&t, "claude");
    let t = crate::fre!(r"\b(?:cloud|clode)\b(?=.*\btmux\b)").replace_all(&t, "claude");
    crate::re!(r"^jarvis ").replace(&t, "").into_owned()
}

/// Touches dites → touches tmux.
pub fn keys() -> &'static [(&'static regex::Regex, &'static [&'static str])] {
    static K: std::sync::LazyLock<Vec<(&'static regex::Regex, &'static [&'static str])>> = std::sync::LazyLock::new(|| {
        vec![
            (crate::re!(r"^(?:appuie sur |touche )?(?:entree|valide|valider|envoie|envoyer)$"), &["Enter"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?(?:echap|echappe|escape)$"), &["Escape"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?(?:controle|ctrl|contrôle) c$"), &["C-c"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?(?:controle|ctrl) d$"), &["C-d"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?(?:tab|tabulation)$"), &["Tab"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?(?:shift|maj|majuscule) tab(?:ulation)?$|^change de mode$"), &["BTab"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?fleche (?:du )?haut$"), &["Up"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?fleche (?:du )?bas$"), &["Down"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?fleche (?:de )?gauche$"), &["Left"][..]),
            (crate::re!(r"^(?:appuie sur |touche )?fleche (?:de )?droite$"), &["Right"][..]),
            (crate::re!(r"^efface (?:la ligne|tout|ce que j ai dit)$"), &["C-u"][..]),
        ]
    });
    &K
}

/// Touches de la dictée (sans Entrée : elle ne part que sur « je valide le prompt »).
pub fn dict_keys() -> Vec<(&'static regex::Regex, &'static [&'static str])> {
    keys().iter().filter(|(_, k)| !(k.len() == 1 && k[0] == "Enter")).cloned().collect()
}

const PROMPT_WORD: &str = "(?:prompt|promt|prompte|promte|prompts|pronte|pront|promptement|prompteur)";
const PROMPT_HEARD: &str = "(?:compte|comptes|prix)";

pub fn validate_re() -> &'static regex::Regex {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        regex::Regex::new(&format!("^(?:(?:ok|bon|alors|donc|allez|et|c est bon) )?(?:je |j )?(?:valide|valides|validez|valider) (?:le |la |ce |mon |ton )?(?:{PROMPT_WORD}|{PROMPT_HEARD}|message|commande)(?: (?:jarvis|s il te plait|stp|merci))*$")).unwrap()
    })
}

pub fn validate_tail_re() -> &'static regex::Regex {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        regex::Regex::new(&format!(r"(?i)[\s,;:.!?-]*(?:et\s+)?(?:je\s+|j['’]\s*)valid(?:e|es|ez|er)\s+(?:le\s+|ce\s+|mon\s+)?{PROMPT_WORD}[\s,.!?]*(?:jarvis[\s,.!?]*)?$")).unwrap()
    })
}

pub fn enter_word_re() -> &'static regex::Regex {
    crate::re!(r"^(?:appuie sur |touche )?(?:entree|valide|valider|envoie|envoyer)$|^(?:je|j) valide$")
}

fn close_relay_re() -> &'static regex::Regex {
    crate::re!(r"^(?:reprends? la main|fin (?:de|du) (?:tmux|relais|la transmission)|(?:arrete|arretes|stop|stoppe) (?:d ecrire|de transmettre|le relais|de parler a claude|d ecrire a claude)(?: dans tmux| a claude)?|quitte (?:le relais|le mode tmux|tmux)|sors de tmux|stop tmux|arrete tmux|c est tout pour claude)$")
}

fn say_err(reason: &str, cfg: &J) -> String {
    match reason {
        "no-distro" => {
            let d = js::gs(cfg, "distro");
            format!("tmux n'est pas prêt : je ne trouve pas la distribution WSL {d}. Installe-la avec la commande wsl, tiret tiret install, tiret d, {d}.")
        }
        "no-tmux" => {
            if js::gs(cfg, "launcher") == "native" {
                "psmux est introuvable. Installe-le avec la commande : winget install psmux.".into()
            } else {
                "tmux n'est pas installé. Dans WSL, lance : sudo apt install tmux.".into()
            }
        }
        "no-session" => "Aucune session tmux n'est ouverte. Dis « ouvre tmux ».".into(),
        "no-window" => "Cette fenêtre tmux n'existe plus.".into(),
        _ => "tmux ne répond pas.".into(),
    }
}

/// « lis l'écran de claude stats ia » → « stats ia », sinon ''.
fn instance_after_claude(t: &str) -> String {
    match crate::re!(r"\bclaude (?:de |du |dans |sur |pour )?(?:le |la )?(?:projet |session )?([a-z0-9][a-z0-9 ]*?)(?: dans tmux)?$").captures(t) {
        Some(m) if !crate::re!(r"^(?:dans tmux|tmux)$").is_match(&m[1]) => m[1].trim().to_string(),
        _ => String::new(),
    }
}

pub fn spoken_number(s: &str) -> String {
    let v = s.trim();
    match v {
        "un" | "une" | "premiere" => "1",
        "deux" | "deuxieme" => "2",
        "trois" | "troisieme" => "3",
        "quatre" => "4",
        "cinq" => "5",
        "six" => "6",
        "sept" => "7",
        "huit" => "8",
        "neuf" => "9",
        "zero" => "0",
        other => other,
    }
    .to_string()
}

/// Lignes utiles d'une capture : sans cadres, barres d'état ni lignes vides de l'interface.
pub fn meaningful_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|l| {
            let l = crate::re!(r"[\u{2500}-\u{257F}\u{2580}-\u{259F}]+").replace_all(l, " ");
            crate::re!(r"\s+").replace_all(&l, " ").trim().to_string()
        })
        .filter(|l| !l.is_empty() && !crate::re!(r"(?i)^(?:>|\? for shortcuts|⏵⏵|bypass permissions)").is_match(l) && crate::re!(r"[\p{L}\p{N}]").is_match(l))
        .collect()
}

#[derive(Clone, Debug)]
pub struct Relay {
    pub target: String,
    pub label: String,
    pub agent_pid: u32,
    pub since: i64,
    pub last_at: i64,
    pub sent: u32,
}

/// Destination d'écriture.
#[derive(Clone)]
pub enum Dest {
    Found { target: String, label: String, agent_pid: u32, instance: Option<J> },
    Ambiguous(Vec<J>),
    Missing(String, Vec<J>),
    None,
}

#[derive(Clone, Default)]
pub struct ClaudeState {
    pub state: String,
    pub idle_runs: u32,
    pub busy_seen: bool,
    pub closed: bool,
    pub at: i64,
}

pub struct TmuxVoice {
    o: OrchRef,
    pub tmux: Rc<Tmux>,
    pub relay: RefCell<Option<Relay>>,
    /// volet → (projet, dossier), ordre d'insertion
    claude_cards: RefCell<Vec<(String, String, String)>>,
    pub claude_at: Cell<i64>,
    pub discovered: RefCell<Option<(i64, Vec<J>)>>,
    panes_sig: RefCell<String>,
    pub claude_states: RefCell<HashMap<String, ClaudeState>>,
    watching_claude: Cell<bool>,
    seen_attached: RefCell<HashSet<String>>,
    app_pids: RefCell<HashMap<String, u32>>,
}

fn pid_of(v: &J) -> u32 {
    v.as_u64().unwrap_or(0) as u32
}

impl TmuxVoice {
    pub fn new(o: OrchRef, tmux: Rc<Tmux>) -> Rc<Self> {
        Rc::new(Self {
            o,
            tmux,
            relay: RefCell::new(None),
            claude_cards: RefCell::new(vec![]),
            claude_at: Cell::new(0),
            discovered: RefCell::new(None),
            panes_sig: RefCell::new(String::new()),
            claude_states: RefCell::new(HashMap::new()),
            watching_claude: Cell::new(false),
            seen_attached: RefCell::new(HashSet::new()),
            app_pids: RefCell::new(HashMap::new()),
        })
    }

    pub fn error_say(&self, e: &TmuxError) -> String {
        self.o.up().log(js::slice_chars(&format!("[tmux] {} : {}", e.reason, if e.tmux.is_empty() { &e.message } else { &e.tmux }), 300));
        say_err(&e.reason, &self.tmux.cfg())
    }

    fn no_session_error() -> TmuxError {
        TmuxError { reason: "no-session".into(), code: -1, tmux: String::new(), message: "pas de session".into() }
    }

    /// Libellé du relais (« Claude · stats-ia »), pour le statut.
    pub fn relay_label(&self) -> Option<String> {
        self.relay.borrow().as_ref().map(|r| r.label.clone())
    }

    /// Dossier d'un projet cité à la voix : projets de l'utilisateur, sinon la zone « projects » sur 3 niveaux.
    pub fn find_project(&self, spoken: &str) -> Option<String> {
        let o = self.o.up();
        if let Some(p) = o.projects.find(spoken) {
            return Some(js::gs(&p, "path"));
        }
        let want = crate::re!(r"^(?:le|la|les|mon|ma) ").replace(&fold(spoken), "").into_owned();
        if want.is_empty() {
            return None;
        }
        let mut roots: Vec<String> = o
            .zones
            .list()
            .iter()
            .filter(|z| js::gs(z, "id") == "projects" || crate::re!(r"(?i)projets?").is_match(&js::gs(z, "name")))
            .map(|z| js::gs(z, "path"))
            .collect();
        if let Some(parent) = crate::paths::workspace().parent() {
            roots.push(parent.to_string_lossy().to_string());
        }
        let mut found: Vec<(String, i32, i32)> = vec![];
        fn walk(dir: &Path, depth: i32, want: &str, found: &mut Vec<(String, i32, i32)>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().to_string();
                if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) || name.starts_with('.') || name == "node_modules" {
                    continue;
                }
                let full = e.path();
                let n = fold(&name);
                let score = if n == want || n.replace(' ', "") == want.replace(' ', "") {
                    3
                } else if n.starts_with(want) {
                    2
                } else if n.contains(want) {
                    1
                } else {
                    0
                };
                if score > 0 {
                    found.push((full.to_string_lossy().to_string(), score, depth));
                }
                if depth < 3 {
                    walk(&full, depth + 1, want, found);
                }
            }
        }
        let mut seen = HashSet::new();
        for r in roots {
            if seen.insert(r.clone()) {
                walk(Path::new(&r), 1, &want, &mut found);
            }
        }
        found.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
        found.into_iter().next().map(|f| f.0)
    }

    /// Action derrière local_tool : { ok: true, … } ou { ok: false, error, reason, say }.
    pub async fn tool<F, Fu>(&self, name: &str, params: J, exec: F) -> J
    where
        F: FnOnce(J) -> Fu + 'static,
        Fu: Future<Output = Result<J, TmuxError>> + 'static,
    {
        let o = self.o.clone();
        self.o
            .up()
            .local_tool(
                name,
                params,
                Box::new(move |p| {
                    Box::pin(async move {
                        match exec(p).await {
                            Ok(v) => {
                                let mut r = json!({ "ok": true });
                                if v.is_object() {
                                    js::assign(&mut r, v);
                                }
                                r
                            }
                            Err(e) => {
                                let say = o.up().tmux_voice.error_say(&e);
                                json!({ "ok": false, "error": e.to_string(), "reason": e.reason, "say": say })
                            }
                        }
                    })
                }),
            )
            .await
    }

    /// Réponse d'un outil : refus de sécurité, échec tmux, ou succès.
    fn outcome(r: &J, say: Option<String>, display: &str) -> J {
        if js::gb(r, "denied") {
            return json!({ "say": "Action refusée ou annulée.", "display": "Refusé" });
        }
        if !js::truthy(r) || r["ok"] == json!(false) {
            return json!({ "say": if js::truthy(&r["say"]) { js::gs(r, "say") } else { "tmux ne répond pas.".into() }, "display": "tmux indisponible" });
        }
        json!({ "say": say, "display": display })
    }

    fn failed(r: &J) -> bool {
        !js::truthy(r) || r["ok"] == json!(false) || js::gb(r, "denied")
    }

    // --- Claude Code déjà ouverts dans psmux ----------------------------------------------------------

    /// Projet (Administration › Projets) auquel appartient un dossier : le plus précis.
    fn project_of(&self, dir: &str) -> Option<String> {
        if dir.is_empty() {
            return None;
        }
        let d = dir.replace('/', "\\").trim_end_matches('\\').to_lowercase();
        let mut best: Option<(String, String)> = None;
        for p in self.o.up().projects.list() {
            let pp = js::gs(&p, "path").replace('/', "\\").trim_end_matches('\\').to_lowercase();
            if (d == pp || d.starts_with(&format!("{pp}\\"))) && best.as_ref().map(|b| pp.len() > b.1.len()).unwrap_or(true) {
                best = Some((js::gs(&p, "name"), pp));
            }
        }
        best.map(|b| b.0)
    }

    /// Instances d'agents dans tous les volets (cache de max_age ms).
    pub async fn discover(&self, max_age: i64) -> Vec<J> {
        if let Some((at, list)) = self.discovered.borrow().as_ref() {
            if js::now_ms() - at < max_age {
                return list.clone();
            }
        }
        let panes = self.tmux.list_panes().await.unwrap_or_default();
        let table = if panes.is_empty() { HashMap::new() } else { tokio::task::spawn_blocking(process_table).await.unwrap_or_default() };
        let mut list: Vec<J> = vec![];
        for p in &panes {
            let pid = pid_of(&p["pid"]);
            let table2 = table.clone();
            let found = tokio::task::spawn_blocking(move || find_agent_process(pid, &table2)).await.ok().flatten();
            let found = found.or_else(|| is_agent(&json!({ "command": p["command"], "title": p["title"] })).then_some(("Claude Code", 0)));
            let Some((agent, agent_pid)) = found else { continue };
            let path = js::gs(p, "path");
            let project = self.project_of(&path);
            let wn = js::gs(p, "windowName");
            let base = Path::new(&path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let label = project
                .clone()
                .or_else(|| wn.strip_prefix("claude-").map(String::from).filter(|s| !s.is_empty()))
                .or_else(|| Some(base).filter(|s| !s.is_empty()))
                .or_else(|| Some(wn.clone()).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| js::gs(p, "target"));
            list.push(json!({
                "target": p["target"], "session": p["session"], "windowName": wn, "path": path,
                "agent": agent, "agentPid": agent_pid, "project": project, "label": label
            }));
        }
        // Deux instances dans le même projet : la session (puis la fenêtre) les distingue.
        let labels: Vec<String> = list.iter().map(|i| js::gs(i, "label")).collect();
        for inst in list.iter_mut() {
            let l = js::gs(inst, "label");
            if labels.iter().filter(|x| **x == l).count() > 1 {
                let t = js::gs(inst, "target");
                let win = t.split(':').nth(1).unwrap_or("").split('.').next().unwrap_or("").to_string();
                inst["label"] = json!(format!("{l} (session {}, fenêtre {win})", js::gs(inst, "session")));
            }
        }
        if list.iter().any(|i| js::gs(i, "agent") == "Claude Code") && self.claude_at.get() == 0 {
            self.claude_at.set(js::now_ms());
        }
        *self.discovered.borrow_mut() = Some((js::now_ms(), list.clone()));
        list
    }

    pub async fn claude_instances(&self, max_age: i64) -> Vec<J> {
        self.discover(max_age).await.into_iter().filter(|i| js::gs(i, "agent") == "Claude Code").collect()
    }

    /// Claude Code connus (dernière détection) et cibles qui attendent une réponse.
    pub fn claude_sessions(&self) -> (Vec<J>, Vec<String>) {
        let list: Vec<J> = self.discovered.borrow().as_ref().map(|(_, l)| l.iter().filter(|i| js::gs(i, "agent") == "Claude Code").cloned().collect()).unwrap_or_default();
        let states = self.claude_states.borrow();
        let asking = list.iter().map(|i| js::gs(i, "target")).filter(|t| states.get(t).map(|s| s.state == "asking").unwrap_or(false)).collect();
        (list, asking)
    }

    /// Instance désignée à la voix : numéro (« le 2 »), projet, fenêtre ou session.
    fn find_instance(&self, list: &[J], spoken: &str) -> Option<J> {
        let s = crate::re!(r"^(?:le|la|du|de|mon|ma|numero|n)\s+").replace(&fold_name(spoken), "").into_owned();
        let s = crate::re!(r"^(?:projet|session|fenetre)\s+").replace(&s, "").trim().to_string();
        if s.is_empty() {
            return None;
        }
        let n = spoken_number(&s);
        if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
            return n.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| list.get(i).cloned());
        }
        let want = name_key(&s);
        let by_project = self.o.up().projects.find(&s);
        let k = |i: &J, f: &str| name_key(&js::gs(i, f));
        list.iter()
            .find(|i| k(i, "label") == want || k(i, "project") == want || k(i, "windowName") == want || k(i, "session") == want)
            .or_else(|| by_project.as_ref().and_then(|bp| list.iter().find(|i| js::gs(i, "project") == js::gs(bp, "name"))))
            .or_else(|| list.iter().find(|i| k(i, "label").starts_with(&want) || k(i, "windowName").starts_with(&want)))
            .cloned()
    }

    /// « stats-ia de corriger le bug » → (instance, « corriger le bug »).
    fn leading_instance(list: &[J], text: &str) -> Option<(J, String)> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut candidates: Vec<(J, String)> = list
            .iter()
            .flat_map(|i| ["label", "project", "windowName"].iter().map(move |f| (i.clone(), js::gs(i, f))).filter(|(_, n)| !n.is_empty()).map(|(i, n)| (i, name_key(&n))))
            .filter(|(_, n)| n.chars().count() >= 3)
            .collect();
        candidates.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        for (inst, name) in candidates {
            let mut acc = String::new();
            for k in 0..words.len().saturating_sub(1).min(6) {
                acc.push_str(&name_key(words[k]));
                if acc == name {
                    let rest = words[k + 1..].join(" ");
                    let rest = crate::re!(r"(?i)^(?:de|d['’]|que|qu['’]|:|,)\s*").replace(&rest, "").trim().to_string();
                    if rest.chars().count() >= 2 {
                        return Some((inst, rest));
                    }
                }
                if acc.len() >= name.len() {
                    break;
                }
            }
        }
        None
    }

    fn dest_of(inst: &J) -> Dest {
        Dest::Found { target: js::gs(inst, "target"), label: format!("Claude · {}", js::gs(inst, "label")), agent_pid: pid_of(&inst["agentPid"]), instance: Some(inst.clone()) }
    }

    /// Application rattachée à une card : le volet psmux de cet agent.
    pub fn app_of(inst: &J) -> J {
        let agent = js::gs(inst, "agent");
        let who = if agent == "Claude Code" || agent.is_empty() { "Claude".to_string() } else { agent };
        json!({ "type": "tmux", "target": inst["target"], "label": format!("{who} · {}", js::gs(inst, "label")), "agentPid": pid_of(&inst["agentPid"]) })
    }

    pub fn card(list: &[J]) -> J {
        let rows: Vec<J> = list
            .iter()
            .enumerate()
            .map(|(k, i)| {
                let label = js::gs(i, "label");
                let path = js::gs(i, "path");
                let agent = js::gs(i, "agent");
                json!({
                    "icon": "◈", "title": format!("{}. {label}", k + 1),
                    "sub": format!("{}{}", js::gs(i, "target"), if path.is_empty() { String::new() } else { format!(" · {path}") }),
                    "meta": if agent != "Claude Code" { agent } else { String::new() },
                    "app": Self::app_of(i),
                    "say": if label.contains("(session ") { format!("Parle à Claude numéro {}", k + 1) } else { format!("Parle à Claude {label}") },
                    "sayLabel": "🎙 Parler"
                })
            })
            .collect();
        let mut card = json!({ "title": "Claude Code ouverts", "tool": "psmux", "count": list.len(), "rows": rows, "footer": "clic : afficher le terminal · 🎙 : lui parler à la voix · × : fermer" });
        if list.len() == 1 {
            card["app"] = Self::app_of(&list[0]);
        }
        card
    }

    /// Phrase qui liste les instances (« stats-ia, jarvis et Shelley »).
    pub fn names(list: &[J]) -> String {
        let n: Vec<String> = list.iter().map(|i| js::gs(i, "label")).collect();
        match n.len() {
            0 => String::new(),
            1 => n[0].clone(),
            l => format!("{} et {}", n[..l - 1].join(", "), n[l - 1]),
        }
    }

    /// Suivi de fond (30 s) : la table des processus n'est relue que si des volets ont changé.
    pub async fn watch(&self) {
        let Ok(panes) = self.tmux.list_panes().await else { return };
        let sig = panes.iter().map(|p| format!("{}|{}|{}|{}", js::gs(p, "target"), js::s(&p["pid"]), js::gs(p, "command"), js::gs(p, "title"))).collect::<Vec<_>>().join("\n");
        if *self.panes_sig.borrow() == sig {
            return;
        }
        *self.panes_sig.borrow_mut() = sig;
        self.discover(0).await;
    }

    // --- Veille des Claude Code ouverts ------------------------------------------------------------

    pub fn screen_state(text: &str) -> &'static str {
        let lines: Vec<&str> = text.split('\n').collect();
        let tail = lines[lines.len().saturating_sub(30)..].join("\n");
        if crate::re!(r"(?i)esc to interrupt|esc pour interrompre|ctrl\+c to interrupt").is_match(&tail) {
            "busy"
        } else if crate::re!(r"(?i)Do you want to|Would you like to|Voulez-vous|Souhaitez-vous|Enter to (?:select|confirm)|❯\s*\d+\.\s|\(y/n\)|\[Y/n\]").is_match(&tail) {
            "asking"
        } else {
            "idle"
        }
    }

    pub fn question(text: &str) -> String {
        let lines: Vec<String> = meaningful_lines(text).into_iter().filter(|l| !crate::re!(r"^(?:❯\s*)?\d+\.\s").is_match(l)).collect();
        let n = lines.len();
        for i in (n.saturating_sub(12)..n).rev() {
            if crate::re!(r"\?\s*$").is_match(&lines[i]) {
                return js::slice_chars(&crate::re!(r"^[●⏺✻*\s]+").replace(&lines[i], ""), 160);
            }
        }
        String::new()
    }

    pub async fn watch_claude(&self) {
        if self.watching_claude.get() {
            return;
        }
        self.watching_claude.set(true);
        self.watch_claude_once().await;
        self.o.up().cards.sweep().await;
        self.watching_claude.set(false);
    }

    /// Application d'une card (volet psmux) encore ouverte ? Some(true/false) ou None (pas encore su).
    pub fn app_alive(&self, app: &J) -> Option<bool> {
        let target = js::gs(app, "target");
        let known = pid_of(&app["agentPid"]).max(self.app_pids.borrow().get(&target).copied().unwrap_or(0));
        if known != 0 {
            return Some(process_alive(known));
        }
        let d = self.discovered.borrow();
        let (at, list) = d.as_ref()?;
        if js::now_ms() - at > 60000 {
            return None;
        }
        if let Some(inst) = list.iter().find(|i| {
            let t = js::gs(i, "target");
            t == target || t.starts_with(&format!("{target}."))
        }) {
            let pid = pid_of(&inst["agentPid"]);
            if pid != 0 {
                self.app_pids.borrow_mut().insert(target, pid);
            }
            return Some(true);
        }
        if js::now_ms() - js::gn_or(app, "since", 0.0) as i64 > 45000 { Some(false) } else { None }
    }

    /// Card d'un volet : retirée quand son application s'arrête, ou quand on ferme le terminal de sa session.
    pub async fn card_alive(&self, app: &J) -> Option<bool> {
        let alive = self.app_alive(app);
        if alive == Some(false) {
            return Some(false);
        }
        let target = js::gs(app, "target");
        let session = target.split(':').next().filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| self.tmux.session());
        if self.tmux.attached(Some(&session)).await {
            self.seen_attached.borrow_mut().insert(target);
            return alive;
        }
        if self.seen_attached.borrow().contains(&target) { Some(false) } else { alive }
    }

    async fn watch_claude_once(&self) {
        let list: Vec<J> = self.discovered.borrow().as_ref().map(|(_, l)| l.iter().filter(|i| js::gs(i, "agent") == "Claude Code").cloned().collect()).unwrap_or_default();
        let mut alive = HashSet::new();
        let o = self.o.up();
        for inst in &list {
            let target = js::gs(inst, "target");
            let label = js::gs(inst, "label");
            alive.insert(target.clone());
            let prev = self.claude_states.borrow().get(&target).cloned();
            let pid = pid_of(&inst["agentPid"]);
            if pid != 0 && !process_alive(pid) {
                if let Some(p) = prev.filter(|p| !p.closed) {
                    self.claude_states.borrow_mut().insert(target.clone(), ClaudeState { closed: true, ..p });
                    let was_relay = self.relay.borrow().as_ref().map(|r| r.target == target).unwrap_or(false);
                    if was_relay {
                        self.stop_relay("agent ferme");
                        o.publish_status();
                    }
                    o.notify(json!({
                        "text": if was_relay { format!("Claude s'est fermé dans {label}. Je reprends la main.") } else { format!("Claude s'est fermé dans {label}.") },
                        "source": "claude", "speak": was_relay
                    }));
                    *self.discovered.borrow_mut() = None;
                }
                continue;
            }
            let Ok(text) = self.tmux.capture_screen(&target).await else { continue };
            let state = Self::screen_state(&text);
            let idle_runs = if state == "idle" { if prev.as_ref().map(|p| p.state == "idle").unwrap_or(false) { prev.as_ref().unwrap().idle_runs + 1 } else { 1 } } else { 0 };
            let mut next = ClaudeState { state: state.into(), idle_runs, busy_seen: state == "busy" || prev.as_ref().map(|p| p.busy_seen).unwrap_or(false), closed: false, at: js::now_ms() };
            let Some(prev) = prev else {
                self.claude_states.borrow_mut().insert(target.clone(), next);
                continue;
            };
            let in_relay = self.relay.borrow().as_ref().map(|r| r.target == target).unwrap_or(false);
            let rows_for = |extra: Vec<J>| {
                let mut v = extra;
                v.push(json!({ "icon": "◈", "title": format!("Parler à Claude {label}"), "say": format!("Parle à Claude {label}") }));
                v.push(json!({ "icon": "▤", "title": "Lire son écran", "say": format!("Lis l'écran de Claude {label} dans tmux") }));
                v
            };
            if state == "asking" && prev.state != "asking" {
                let q = Self::question(&text);
                let end = if !q.is_empty() && crate::re!(r"[?!.]$").is_match(&q) { "" } else { "." };
                let qq = if q.is_empty() { String::new() } else { format!(" : {q}") };
                o.notify(json!({
                    "text": if in_relay { format!("Claude te pose une question{qq}{end}") } else { format!("Claude, dans {label}, attend ta réponse{qq}{end}") },
                    "title": format!("Claude · {label}"), "source": "claude",
                    "rows": rows_for(if q.is_empty() { vec![] } else { vec![json!({ "icon": "?", "tone": "warn", "title": q, "sub": "question de Claude" })] }),
                    "app": Self::app_of(inst)
                }));
            } else if state == "idle" && idle_runs == 2 && prev.busy_seen {
                let ml = meaningful_lines(&text);
                let last: Vec<String> = ml[ml.len().saturating_sub(2)..].iter().map(|l| crate::re!(r"^[●⏺✻*\s]+").replace(l, "").into_owned()).collect();
                let last = last.join(" ");
                let n = last.chars().count();
                let last: String = last.chars().skip(n.saturating_sub(220)).collect();
                o.notify(json!({
                    "text": if in_relay { "Claude a terminé.".to_string() } else { format!("Claude a terminé dans {label}.") },
                    "title": format!("Claude · {label}"), "source": "claude",
                    "rows": rows_for(if last.is_empty() { vec![] } else { vec![json!({ "icon": "✓", "tone": "ok", "title": last, "sub": "dernières lignes" })] }),
                    "app": Self::app_of(inst)
                }));
                next.busy_seen = false;
                self.claude_states.borrow_mut().insert(target.clone(), next);
                let all_idle = self.claude_states.borrow().values().all(|s| s.closed || s.state != "busy");
                o.schedules.on_claude_done(&label, all_idle);
                continue;
            }
            self.claude_states.borrow_mut().insert(target, next);
        }
        self.claude_states.borrow_mut().retain(|t, _| alive.contains(t));
    }

    /// Au démarrage : les Claude Code déjà ouverts dans psmux sont annoncés.
    pub async fn announce(&self) {
        let list = self.claude_instances(0).await;
        if list.is_empty() {
            return;
        }
        let o = self.o.up();
        o.record("tmux_detection", json!({ "instances": list.iter().map(|i| json!({ "cible": i["target"], "nom": i["label"], "dossier": i["path"] })).collect::<Vec<_>>() }));
        o.show_widget("tool", Self::card(&list));
        let text = if list.len() == 1 {
            format!("Claude Code tourne dans psmux, dans {}. Dis « parle à Claude » pour lui écrire à la voix.", js::gs(&list[0], "label"))
        } else {
            format!("{} Claude Code tournent dans psmux : {}. Dis par exemple « parle à Claude {} ».", list.len(), Self::names(&list), js::gs(&list[0], "label"))
        };
        o.emit(json!({ "type": "say", "text": text }));
    }

    /// Cible d'écriture.
    pub async fn target(&self, want_claude: bool, spoken: &str) -> Result<Dest, TmuxError> {
        if want_claude {
            let list = self.claude_instances(10000).await;
            if !spoken.is_empty() {
                return Ok(match self.find_instance(&list, spoken) {
                    Some(i) => Self::dest_of(&i),
                    None => Dest::Missing(spoken.to_string(), list),
                });
            }
            if list.len() == 1 {
                return Ok(Self::dest_of(&list[0]));
            }
            if list.len() > 1 {
                return Ok(Dest::Ambiguous(list));
            }
            if self.tmux.has_session().await.unwrap_or(false) {
                if let Some(w) = self.tmux.windows().await?.into_iter().find(is_claude) {
                    return Ok(Dest::Found { target: js::gs(&w, "target"), label: format!("Claude (fenêtre {})", js::s(&w["index"])), agent_pid: 0, instance: None });
                }
            }
            return Ok(Dest::None);
        }
        if !self.tmux.has_session().await? {
            return Err(Self::no_session_error());
        }
        Ok(Dest::Found { target: format!("{}:", self.tmux.session()), label: "tmux".into(), agent_pid: 0, instance: None })
    }

    /// Réponse quand la cible Claude n'est pas unique ou introuvable (None si la cible est bonne).
    fn target_problem(dest: &Dest) -> Option<J> {
        match dest {
            Dest::Found { .. } => None,
            Dest::None => Some(json!({ "say": "Aucun Claude Code ne tourne dans psmux. Dis « lance Claude Code dans le projet » suivi du nom.", "display": "Pas de Claude" })),
            Dest::Ambiguous(l) => Some(json!({
                "say": format!("{} Claude Code sont ouverts : {}. Lequel ? Dis par exemple « parle à Claude {} ».", l.len(), Self::names(l), js::gs(&l[0], "label")),
                "display": "Plusieurs Claude"
            })),
            Dest::Missing(m, l) => Some(if l.is_empty() {
                json!({ "say": "Aucun Claude Code ne tourne dans psmux.", "display": "Pas de Claude" })
            } else {
                json!({ "say": format!("Je ne vois pas de Claude Code pour {m}. Ceux qui sont ouverts : {}.", Self::names(l)), "display": "Claude introuvable" })
            }),
        }
    }

    /// Écrit une phrase dans un volet. Agent : libre ; shell ou autre programme : code exigé.
    pub async fn write(&self, target: &str, text: &str, known: Option<(u32, String)>) -> J {
        let tm = self.tmux.clone();
        let (t, x) = (target.to_string(), text.to_string());
        if let Some((pid, label)) = known.filter(|(p, _)| *p != 0 && process_alive(*p)) {
            let _ = pid;
            return self.tool("mcp__system__tmux_dicter", json!({ "texte": text, "cible": label }), move |_| async move { tm.send_line(&t, &x, true).await.map(|_| Value::Null) }).await;
        }
        let info = match self.tmux.pane_info(target).await {
            Ok(i) => i,
            Err(e) => return json!({ "ok": false, "say": self.error_say(&e) }),
        };
        let tool = if is_agent(&info) { "mcp__system__tmux_dicter" } else { "mcp__system__tmux_shell" };
        let name = if js::truthy(&info["name"]) { js::gs(&info, "name") } else { target.to_string() };
        self.tool(tool, json!({ "texte": text, "cible": format!("{name} · {}", js::gs(&info, "command")) }), move |_| async move { tm.send_line(&t, &x, true).await.map(|_| Value::Null) }).await
    }

    fn intent<F, Fu>(&self, kind: &str, label: &str, detail: &str, f: F) -> Intent
    where
        F: Fn(Rc<TmuxVoice>) -> Fu + 'static,
        Fu: Future<Output = J> + 'static,
    {
        let o = self.o.clone();
        let f = Rc::new(f);
        Intent::new(&format!("tmux_{kind}"), label, detail, move || {
            let (o, f) = (o.clone(), f.clone());
            async move {
                let tv = o.up().tmux_voice.clone();
                f(tv).await
            }
        })
    }

    /// Phrase adressée à tmux (ou, pendant le relais, commande de navigation). None sinon.
    pub fn match_(&self, raw_text: &str, relay: bool) -> Option<Intent> {
        let t = fold(raw_text);
        let raw = crate::re!(r"(?i)^jarvis[\s,]+").replace(raw_text.trim(), "").into_owned();
        let has_tmux = crate::re!(r"\btmux\b").is_match(&t);
        let session = self.tmux.session();

        // Claude Code dans tmux, éventuellement dans un projet.
        if let Some(m) = crate::re!(r"^(?:(?:dans|sur) tmux )?(?:lance|lancer|ouvre|demarre|mets|relance)(?: moi)?(?: un| une| le| la)?(?: nouveau| nouvelle| autre)?(?: session| instance)?(?: de)? claude\b(.*)$").captures(&t) {
            if has_tmux || relay {
                let rest = m.get(1).map(|x| x.as_str()).unwrap_or("");
                let spoken = crate::re!(r"(?:dans|sur|pour) (?:le |mon |ma )?(?:projet|dossier|repertoire) (.+?)(?: (?:dans|sur) tmux)?$").captures(rest).map(|c| c[1].trim().to_string());
                let dir = spoken.as_ref().and_then(|s| self.find_project(s));
                let detail = spoken.clone().unwrap_or_else(|| "dossier par défaut".into());
                return Some(self.intent("claude", "Claude Code · tmux", &detail, move |tv| {
                    let (spoken, dir) = (spoken.clone(), dir.clone());
                    async move {
                        if let (Some(s), None) = (&spoken, &dir) {
                            return json!({ "say": format!("Je ne trouve pas de projet {s}."), "display": "Projet introuvable" });
                        }
                        let (r, name, command) = tv.launch_claude(dir.as_deref()).await;
                        let base = dir.as_ref().and_then(|d| Path::new(d).file_name().map(|f| f.to_string_lossy().to_string()));
                        Self::outcome(
                            &r,
                            Some(format!("Claude Code est lancé dans tmux{}. Dis « parle à Claude » pour que je lui transmette tout ce que tu dis.", base.map(|b| format!(", dans {b}")).unwrap_or_default())),
                            &format!("{name} · {command}"),
                        )
                    }
                }));
            }
        }

        // Claude Code ouverts dans psmux : « quels Claude sont ouverts ? »
        if crate::re!(r"^(?:quels?|quelles?|combien de|liste(?: moi)?(?: les)?|montre moi les|affiche les)(?: sont les)? (?:claude|claudes|sessions? (?:de )?claude|instances? (?:de )?claude|sessions? (?:tmux|psmux))(?: (?:sont |tournent |)?(?:ouverts?|ouvertes?|lances?|lancees?|en cours|actifs?|actives?|detectes?|tournent))?$").is_match(&t)
            || crate::re!(r"^(?:y a t il|est ce qu il y a|est ce que) (?:des |un )?claude (?:ouverts?|lances?|en cours|qui tournent?)(?: dans (?:tmux|psmux))?$").is_match(&t)
        {
            return Some(self.intent("instances", "Claude Code ouverts", "psmux", |tv| async move {
                let list = tv.claude_instances(0).await;
                if list.is_empty() {
                    return json!({ "say": "Aucun Claude Code ne tourne dans psmux.", "display": "Aucun" });
                }
                tv.o.up().show_widget("tool", Self::card(&list));
                json!({
                    "say": if list.len() == 1 {
                        format!("Un Claude Code est ouvert, dans {}. Dis « parle à Claude » pour lui écrire.", js::gs(&list[0], "label"))
                    } else {
                        format!("{} Claude Code sont ouverts : {}. Dis « parle à Claude » suivi du nom.", list.len(), Self::names(&list))
                    },
                    "display": list.iter().map(|i| js::gs(i, "label")).collect::<Vec<_>>().join(" · ")
                })
            }));
        }

        // Relais : chaque phrase suivante est tapée dans tmux.
        if let Some(m) = crate::re!(r"^(?:parle|parler|ecris|ecrire|dicte|transmets|connecte toi|branche toi)(?: directement| tout)? (?:a|avec|dans|sur|au) (claude|tmux|le terminal)(?: dans tmux)?(?: (?:de |du |dans |sur |pour |numero |n )?(?:le |la |mon |ma )?(?:projet |session |fenetre )?(.+?))?(?: dans tmux)?$|^mode (tmux|claude|relais)(?: tmux)?$").captures(&t) {
            let which = m.get(1).or(m.get(3)).map(|x| x.as_str()).unwrap_or("");
            let want_claude = which.contains("claude");
            let spoken = if want_claude { m.get(2).map(|x| x.as_str().trim().to_string()).unwrap_or_default() } else { String::new() };
            let detail = if want_claude { format!("Claude Code{}", if spoken.is_empty() { String::new() } else { format!(" · {spoken}") }) } else { "fenêtre affichée".into() };
            return Some(self.intent("relay", "Relais tmux", &detail, move |tv| {
                let spoken = spoken.clone();
                async move {
                    let dest = match tv.target(want_claude, &spoken).await {
                        Ok(d) => d,
                        Err(e) => return json!({ "say": tv.error_say(&e), "display": "tmux indisponible" }),
                    };
                    if let Some(p) = Self::target_problem(&dest) {
                        return p;
                    }
                    let Dest::Found { target, label, agent_pid, instance } = dest else { return Value::Null };
                    let already = tv.relay.borrow().as_ref().map(|r| r.target == target).unwrap_or(false);
                    if already {
                        if let Some(r) = tv.relay.borrow_mut().as_mut() {
                            r.last_at = js::now_ms();
                        }
                        return json!({ "say": "Le relais est déjà actif : parle, j’écris.", "display": format!("Relais → {label}") });
                    }
                    tv.start_relay(&target, &label, agent_pid);
                    let free = tv.o.up().status.d.borrow().listening_mode == "free";
                    let who = if want_claude {
                        match &instance {
                            Some(i) => format!("Claude, dans {}", js::gs(i, "label")),
                            None => "Claude".into(),
                        }
                    } else {
                        "tmux".into()
                    };
                    json!({
                        "say": format!("Je te passe {who} : tout ce que tu dis y sera écrit, sans l’envoyer. Dis « je valide le prompt » pour l’envoyer, « Jarvis, reprends la main » pour arrêter.{}", if free { "" } else { " Dis « Jarvis, écoute-moi » pour ne plus avoir à dire Jarvis." }),
                        "display": format!("Relais → {label}")
                    })
                }
            }));
        }

        // Une seule phrase envoyée : « dis à Claude de … », « écris dans tmux : … ».
        if let Some(m) = crate::re!(r"(?i)^(?:écris|ecris|tape|envoie|dis|demande|transmets)\s+(?:à|a|au|dans|sur)\s+(claude(?:\s+code)?|cloud\s+code|t-?mux|le\s+terminal)\s*[,:]?\s*(?:de\s+|d['’]\s*|que\s+)?(.{2,})$").captures(&raw) {
            let want_claude = crate::re!(r"(?i)claude|cloud").is_match(&m[1]);
            let said = m[2].trim().to_string();
            return Some(self.intent("send", if want_claude { "Message à Claude" } else { "Saisie tmux" }, &js::slice_chars(&said, 40), move |tv| {
                let said = said.clone();
                async move {
                    let mut text = said.clone();
                    let lead = if want_claude { Self::leading_instance(&tv.claude_instances(10000).await, &said) } else { None };
                    let dest = match lead {
                        Some((inst, rest)) => {
                            text = rest;
                            Self::dest_of(&inst)
                        }
                        None => match tv.target(want_claude, "").await {
                            Ok(d) => d,
                            Err(e) => return json!({ "say": tv.error_say(&e), "display": "tmux indisponible" }),
                        },
                    };
                    if want_claude {
                        if let Some(p) = Self::target_problem(&dest) {
                            return p;
                        }
                    }
                    let Dest::Found { target, label, agent_pid, .. } = dest else { return json!({ "say": "Aucune session tmux n'est ouverte.", "display": "Pas de tmux" }) };
                    let known = (agent_pid != 0).then(|| (agent_pid, label.clone()));
                    let r = tv.write(&target, &text, known).await;
                    Self::outcome(&r, None, &format!("→ {label}"))
                }
            }));
        }

        // Commande quelconque dans une nouvelle fenêtre (code exigé).
        if let Some(m) = crate::re!(r"(?i)^(?:lance|exécute|execute|tape)\s+(?:la\s+commande\s+)?(.+?)\s+(?:dans|sur)\s+(?:un\s+nouveau\s+|une\s+nouvelle\s+)?(?:fenêtre\s+|fenetre\s+|terminal\s+)?t-?mux[.!]?$").captures(&raw) {
            if !crate::re!(r"(?i)^(?:claude|cloud)\b").is_match(&m[1]) {
                let c = m[1].trim();
                let command = {
                    let mut ch = c.chars();
                    match ch.next() {
                        Some(f) => f.to_lowercase().collect::<String>() + ch.as_str(),
                        None => String::new(),
                    }
                };
                return Some(self.intent("command", "Commande tmux", &command.clone(), move |tv| {
                    let command = command.clone();
                    async move {
                        let tm = tv.tmux.clone();
                        let c2 = command.clone();
                        let r = tv
                            .tool("mcp__system__tmux_shell", json!({ "texte": command, "cible": "nouvelle fenêtre" }), move |_| async move {
                                let name = js::slice_chars(c2.split_whitespace().next().unwrap_or(""), 20);
                                let target = tm.new_window(&name, "", &c2).await?;
                                if js::gb(&tm.cfg(), "showTerminal") && !tm.attached(None).await {
                                    tm.open_terminal(None);
                                }
                                Ok(json!({ "target": target }))
                            })
                            .await;
                        Self::outcome(&r, Some("C'est lancé dans tmux.".into()), &command)
                    }
                }));
            }
        }

        // Après « lance Claude Code dans tmux », « lis l'écran de Claude » et « interromps Claude » visent ce Claude.
        let claude_only = !has_tmux && !relay && self.claude_at.get() > 0 && crate::re!(r"\bclaude\b").is_match(&t);
        if !has_tmux && !relay && !claude_only {
            return None;
        }

        // Lire ce qu'affiche le terminal (ou Claude).
        if crate::re!(r"^(?:lis|lire|relis|dis moi|que dit|qu est ce que dit|qu est ce que repond|que repond|ou en est|qu affiche|qu est ce qu affiche|qu est ce qu il y a (?:dans|sur)|montre moi)\b").is_match(&t)
            && (has_tmux || crate::re!(r"\bclaude\b").is_match(&t) || crate::re!(r"\b(?:l ecran|la sortie|le terminal)\b").is_match(&t))
        {
            let want_claude = crate::re!(r"\bclaude\b").is_match(&t);
            let named = if want_claude { instance_after_claude(&t) } else { String::new() };
            let detail = if want_claude { format!("Claude{}", if named.is_empty() { String::new() } else { format!(" · {named}") }) } else { "fenêtre affichée".into() };
            return Some(self.intent("read", "Lecture tmux", &detail, move |tv| {
                let named = named.clone();
                async move {
                    let dest = match tv.target(want_claude, &named).await {
                        Ok(d) => d,
                        Err(e) => return json!({ "say": tv.error_say(&e), "display": "tmux indisponible" }),
                    };
                    if want_claude {
                        if let Some(p) = Self::target_problem(&dest) {
                            return p;
                        }
                    }
                    let Dest::Found { target, label, .. } = dest else { return json!({ "say": "tmux ne répond pas.", "display": "tmux indisponible" }) };
                    let tm = tv.tmux.clone();
                    let t2 = target.clone();
                    let r = tv.tool("mcp__system__tmux", json!({ "action": "lire" }), move |_| async move { Ok(json!({ "text": tm.capture(&t2, 80).await? })) }).await;
                    if Self::failed(&r) {
                        return Self::outcome(&r, None, "");
                    }
                    let lines = meaningful_lines(&js::gs(&r, "text"));
                    let shown = lines[lines.len().saturating_sub(30)..].join("\n");
                    tv.o.up().show_widget("tool", json!({ "title": format!("tmux · {label}"), "tool": "capture du terminal", "result": if shown.is_empty() { "(vide)".to_string() } else { shown } }));
                    let last = lines[lines.len().saturating_sub(3)..].join(" ");
                    let n = last.chars().count();
                    let last: String = last.chars().skip(n.saturating_sub(300)).collect();
                    json!({ "say": if last.is_empty() { "L'écran est vide.".to_string() } else { format!("À l'écran : {last}") }, "display": format!("{} lignes", lines.len()) })
                }
            }));
        }

        // Interrompre Claude (Échap) ou le programme affiché (Ctrl+C).
        if (crate::re!(r"^(?:interromps|interrompt|stoppe|arrete|coupe) (?:claude(?: [a-z0-9 ]+?)?|le programme|la commande)(?: dans tmux)?$").is_match(&t) && !(claude_only && t.starts_with("arrete")))
            || (relay && crate::re!(r"^(?:interromps|interrompt|stop|stoppe|arrete)$").is_match(&t))
        {
            let relay_claude = self.relay.borrow().as_ref().map(|r| crate::re!(r"(?i)claude").is_match(&r.label)).unwrap_or(false);
            let want_claude = t.contains("claude") || relay_claude;
            let named = if t.contains("claude") { instance_after_claude(&t) } else { String::new() };
            return Some(self.intent("interrupt", "Interruption", if want_claude { "Claude (Échap)" } else { "Ctrl+C" }, move |tv| {
                let named = named.clone();
                async move {
                    let current = if named.is_empty() { tv.relay.borrow().as_ref().map(|r| r.target.clone()) } else { None };
                    let target = match current {
                        Some(t) => Some(t),
                        None => {
                            let dest = match tv.target(want_claude, &named).await {
                                Ok(d) => d,
                                Err(e) => return json!({ "say": tv.error_say(&e), "display": "tmux indisponible" }),
                            };
                            if want_claude {
                                if let Some(p) = Self::target_problem(&dest) {
                                    return p;
                                }
                            }
                            match dest {
                                Dest::Found { target, .. } => Some(target),
                                _ => None,
                            }
                        }
                    };
                    let tm = tv.tmux.clone();
                    let r = tv
                        .tool("mcp__system__tmux", json!({ "action": "interrompre" }), move |_| async move {
                            if let Some(t) = target {
                                tm.send_keys(&t, &[if want_claude { "Escape" } else { "C-c" }]).await?;
                            }
                            Ok(Value::Null)
                        })
                        .await;
                    Self::outcome(&r, None, "interrompu")
                }
            }));
        }
        if claude_only {
            return None;
        }

        // Ouvrir / afficher la session.
        if crate::re!(r"^(?:(?:ouvre|lance|demarre|affiche|montre|reviens sur|va sur|va dans)(?: moi)?(?: le| la| une| ma)?(?: session| fenetre| terminal)?(?: de)? )?tmux$").is_match(&t) {
            return Some(self.intent("open", "tmux", &format!("session {session}"), |tv| async move {
                let tm = tv.tmux.clone();
                let s = tm.session();
                let r = tv
                    .tool("mcp__system__tmux", json!({ "action": "ouvrir" }), move |_| async move {
                        tm.ensure_session("").await?;
                        if !tm.attached(None).await {
                            tm.open_terminal(None);
                        } else {
                            let _ = crate::orch::system::call("activer_fenetre", &json!({ "titre": format!("tmux · {}", tm.session()) })).await;
                        }
                        Ok(Value::Null)
                    })
                    .await;
                Self::outcome(&r, Some("tmux est ouvert.".into()), &format!("session {s}"))
            }));
        }

        // Pendant le relais, sans « tmux », seules des formes courtes sont de la navigation.
        let short = |s: &str| has_tmux || s.split(' ').count() <= 2;

        // Fenêtres : liste, navigation, création.
        if (has_tmux && crate::re!(r"\b(?:quelles?|liste|montre|affiche|combien)\b.*\bfenetres?\b").is_match(&t)) || crate::re!(r"^(?:quelles sont les |liste les )?fenetres(?: tmux)?$").is_match(&t) {
            return Some(self.intent("windows", "Fenêtres tmux", "liste", |tv| async move {
                let tm = tv.tmux.clone();
                let r = tv.tool("mcp__system__tmux", json!({ "action": "lister" }), move |_| async move { Ok(json!({ "list": tm.windows().await? })) }).await;
                if Self::failed(&r) {
                    return Self::outcome(&r, None, "");
                }
                let list = js::arr(&r["list"]);
                let names: Vec<String> = list.iter().map(|w| format!("{}, {}{}", js::s(&w["index"]), js::gs(w, "name"), if js::gb(w, "active") { " (affichée)" } else { "" })).collect();
                json!({
                    "say": if list.is_empty() { "Aucune fenêtre.".to_string() } else { format!("{} fenêtre{} : {}.", list.len(), if list.len() > 1 { "s" } else { "" }, names.join(" ; ")) },
                    "display": names.join(" · ")
                })
            }));
        }
        if let Some(m) = crate::re!(r"^(?:fenetre|onglet) (suivante|precedente)(?: tmux| de tmux| dans tmux)?$").captures(&t) {
            let which = m[1].to_string();
            return Some(self.intent("select", "Fenêtre tmux", &which.clone(), move |tv| {
                let which = which.clone();
                async move {
                    let tm = tv.tmux.clone();
                    let next = which == "suivante";
                    let r = tv.tool("mcp__system__tmux", json!({ "action": format!("fenêtre {which}") }), move |_| async move { tm.select_relative(next).await.map(|_| Value::Null) }).await;
                    Self::outcome(&r, None, &which)
                }
            }));
        }
        if let Some(m) = crate::re!(r"^(?:(?:passe|va|bascule|reviens|affiche|montre)(?: moi)?(?: a| sur| dans| vers)? )?(?:la )?fenetre (?:tmux )?(?:numero )?(.+?)(?: (?:de|dans|sur) tmux)?$").captures(&t) {
            let p = m[1].to_string();
            if short(&p) && !crate::re!(r"^(?:tmux|suivante|precedente)$").is_match(&p) && !crate::re!(r"^(?:ferme|tue|supprime)").is_match(&t) {
                let spoken = spoken_number(&p);
                return Some(self.intent("select", "Fenêtre tmux", &spoken.clone(), move |tv| {
                    let spoken = spoken.clone();
                    async move {
                        let tm = tv.tmux.clone();
                        let s2 = spoken.clone();
                        let r = tv
                            .tool("mcp__system__tmux", json!({ "action": format!("fenêtre {spoken}") }), move |_| async move {
                                let found = tm.find_window(&s2).await?;
                                if let Some(f) = &found {
                                    tm.select(&js::gs(f, "target")).await?;
                                }
                                Ok(json!({ "found": found }))
                            })
                            .await;
                        if Self::failed(&r) {
                            return Self::outcome(&r, None, "");
                        }
                        let found = &r["found"];
                        if !found.is_object() {
                            return json!({ "say": format!("Pas de fenêtre {spoken} dans tmux."), "display": "Introuvable" });
                        }
                        if let Some(rl) = tv.relay.borrow_mut().as_mut() {
                            rl.target = js::gs(found, "target");
                            rl.label = js::gs(found, "name");
                        }
                        json!({ "say": null, "display": format!("{} · {}", js::s(&found["index"]), js::gs(found, "name")) })
                    }
                }));
            }
        }
        if crate::re!(r"^(?:(?:ouvre|cree|lance|ajoute)(?: moi)? )?(?:une )?(?:nouvelle fenetre|nouvel onglet)(?: (?:dans|de|sur))?(?: tmux)?$").is_match(&t) {
            return Some(self.intent("window", "Nouvelle fenêtre tmux", "shell", |tv| async move {
                let tm = tv.tmux.clone();
                let r = tv
                    .tool("mcp__system__tmux", json!({ "action": "nouvelle fenêtre" }), move |_| async move {
                        tm.new_window("", "", "").await?;
                        if js::gb(&tm.cfg(), "showTerminal") && !tm.attached(None).await {
                            tm.open_terminal(None);
                        }
                        Ok(Value::Null)
                    })
                    .await;
                Self::outcome(&r, None, "nouvelle fenêtre")
            }));
        }
        if let Some(m) = crate::re!(r"^(?:divise|coupe|partage|separe|scinde)(?: l ecran| la fenetre| le terminal)?(?: de| dans)? tmux(?: en deux)?(?: (cote a cote|a droite|verticalement|en dessous|en bas|horizontalement))?$|^nouveau volet(?: tmux)?(?: (a droite|en dessous|en bas))?$").captures(&t) {
            let where_ = m.get(1).or(m.get(2)).map(|x| x.as_str()).unwrap_or("a droite");
            let side = crate::re!(r"droite|cote|verticalement").is_match(where_);
            let label = if side { "à droite" } else { "en dessous" };
            return Some(self.intent("split", "Volet tmux", label, move |tv| async move {
                let tm = tv.tmux.clone();
                let label = if side { "à droite" } else { "en dessous" };
                let r = tv.tool("mcp__system__tmux", json!({ "action": format!("volet {label}") }), move |_| async move { tm.split(&format!("{}:", tm.session()), side).await.map(|_| Value::Null) }).await;
                Self::outcome(&r, None, label)
            }));
        }

        // Fermer une fenêtre, ou toute la session (accord oral).
        if let Some(m) = crate::re!(r"^(?:ferme|tue|supprime) (?:la )?fenetre (?:tmux )?(.+?)(?: (?:de|dans) tmux)?$").captures(&t) {
            if &m[1] != "tmux" && short(&m[1]) {
                let spoken = spoken_number(&m[1]);
                return Some(self.intent("close", "Fermeture fenêtre tmux", &spoken.clone(), move |tv| {
                    let spoken = spoken.clone();
                    async move {
                        let Some(found) = tv.tmux.find_window(&spoken).await.ok().flatten() else { return json!({ "say": format!("Pas de fenêtre {spoken} dans tmux."), "display": "Introuvable" }) };
                        let (target, name) = (js::gs(&found, "target"), js::gs(&found, "name"));
                        let tm = tv.tmux.clone();
                        let t2 = target.clone();
                        let r = tv.tool("mcp__system__tmux_fermer", json!({ "cible": format!("fenêtre {} ({name})", js::s(&found["index"])) }), move |_| async move { tm.kill_window(&t2).await.map(|_| Value::Null) }).await;
                        if js::gb(&r, "ok") && tv.relay.borrow().as_ref().map(|rl| rl.target == target).unwrap_or(false) {
                            *tv.relay.borrow_mut() = None;
                        }
                        Self::outcome(&r, Some(format!("Fenêtre {name} fermée.")), &name)
                    }
                }));
            }
        }
        if crate::re!(r"^(?:ferme|tue|quitte|arrete) (?:la session |toute la session )?tmux$").is_match(&t) {
            return Some(self.intent("kill", "Fermeture tmux", &format!("session {session}"), |tv| async move {
                let tm = tv.tmux.clone();
                let s = tm.session();
                let r = tv.tool("mcp__system__tmux_fermer", json!({ "cible": format!("session {s} et tout ce qui y tourne") }), move |_| async move { tm.kill_session().await.map(|_| Value::Null) }).await;
                Self::outcome(&r, Some("Session tmux fermée.".into()), &format!("session {s}"))
            }));
        }
        None
    }

    /// Nouvelle fenêtre tmux avec Claude Code, dans `dir` (ou le dossier par défaut).
    pub async fn launch_claude(&self, dir: Option<&str>) -> (J, String, String) {
        let tm = self.tmux.clone();
        let base = dir.and_then(|d| Path::new(d).file_name().map(|f| f.to_string_lossy().to_string()));
        let name = match &base {
            Some(b) => js::slice_chars(&format!("claude-{b}"), 30),
            None => "claude".into(),
        };
        let cfg = tm.cfg();
        let command = js::gs(&cfg, "claudeCommand");
        if command.contains("--dangerously-skip-permissions") {
            self.o.up().record("agent_sans_validation", json!({ "origine": "tmux", "projet": name }));
        }
        *self.discovered.borrow_mut() = None;
        let default_dir = js::gs(&cfg, "defaultDir");
        let shown_dir = dir.map(String::from).unwrap_or_else(|| if default_dir.is_empty() { "~".into() } else { default_dir });
        let (n2, d2, c2) = (name.clone(), dir.unwrap_or("").to_string(), command.clone());
        let r = self
            .tool("mcp__system__tmux_claude", json!({ "commande": command, "dossier": shown_dir }), move |_| async move {
                let target = tm.new_window(&n2, &d2, &c2).await?;
                if js::gb(&tm.cfg(), "showTerminal") && !tm.attached(None).await {
                    tm.open_terminal(None);
                }
                Ok(json!({ "target": target }))
            })
            .await;
        if js::gb(&r, "ok") {
            self.claude_at.set(js::now_ms());
            let target = js::gs(&r, "target");
            if !target.is_empty() {
                let project = base.unwrap_or_else(|| "claude".into());
                self.claude_cards.borrow_mut().retain(|c| c.0 != target);
                self.claude_cards.borrow_mut().push((target.clone(), project, dir.unwrap_or("").to_string()));
                self.show_claude_card(&target);
            }
        }
        (r, name, command)
    }

    /// Une card « Claude · projet » montre-t-elle déjà un volet de cette session ?
    pub fn has_claude_card_in(&self, session: &str) -> bool {
        self.claude_cards.borrow().iter().any(|c| c.0.split(':').next() == Some(session))
    }

    /// Ce que dit l'utilisateur part-il dans ce volet ?
    pub fn dictating(&self, target: &str) -> bool {
        if self.relay.borrow().as_ref().map(|r| r.target == target).unwrap_or(false) {
            return true;
        }
        let session = target.split(':').next().unwrap_or("");
        self.o.up().terminal_voice.target_tmux_session().map(|s| s == session).unwrap_or(false)
    }

    pub fn show_claude_card(&self, target: &str) {
        let Some((_, project, dir)) = self.claude_cards.borrow().iter().find(|c| c.0 == target).cloned() else { return };
        let active = self.dictating(target);
        let known_pid = self.discovered.borrow().as_ref().and_then(|(_, l)| {
            l.iter().find(|i| {
                let t = js::gs(i, "target");
                t == target || t.starts_with(&format!("{target}."))
            }).map(|i| pid_of(&i["agentPid"]))
        });
        let mut app = json!({ "type": "tmux", "target": target, "label": format!("Claude · {project}") });
        if let Some(p) = known_pid.filter(|p| *p != 0) {
            app["agentPid"] = json!(p);
        }
        let o = self.o.up();
        let mut row = json!({
            "icon": "◈", "title": project, "app": app, "voice": { "active": active },
            "sub": if active { "🎙 Dictée : écrit dans Claude sans « Jarvis », envoyé sur « je valide le prompt »".to_string() } else { format!("{target}{}", if dir.is_empty() { String::new() } else { format!(" · {dir}") }) }
        });
        if active {
            row["tone"] = json!("ok");
        }
        let mut rows = vec![row];
        rows.extend(o.terminal_voice.ai_watch_rows());
        rows.extend(o.terminal_voice.action_rows(&app));
        o.show_widget(
            "tool",
            json!({
                "title": format!("Claude · {project}"), "tool": "psmux", "persist": true, "app": app, "rows": rows,
                "footer": "clic : afficher le terminal · 🎙 Dicter ici : ta voix écrite dans Claude · « je valide le prompt » : envoyer · ✋ : Jarvis reprend la main · 🔒 la card part avec Claude · Actions : Administration › Terminal"
            }),
        );
    }

    /// Cards de Claude encore affichées, redessinées ; les fermées sont oubliées.
    pub fn refresh_cards(&self) {
        let cards = self.claude_cards.borrow().clone();
        let o = self.o.up();
        for (target, project, _) in cards {
            if !o.cards.has_key(&format!("tool:Claude · {project}")) {
                self.claude_cards.borrow_mut().retain(|c| c.0 != target);
            } else {
                self.show_claude_card(&target);
            }
        }
    }

    /// Boutons de la card : « 🎙 Dicter ici », « ✋ Jarvis reprend la main ».
    pub async fn card_action(&self, app: &J, action: &str) -> J {
        let o = self.o.up();
        let say = |text: String| o.emit(json!({ "type": "say", "text": text }));
        if crate::re!(r"^(?:act|confirm):").is_match(action) {
            return o.terminal_voice.card_action_button(app, action).await;
        }
        let target = js::gs(app, "target");
        let label = js::gs(app, "label");
        if action == "dicter" {
            let already = self.relay.borrow().as_ref().map(|r| r.target == target).unwrap_or(false);
            if !already {
                self.start_relay(&target, &label, pid_of(&app["agentPid"]));
            }
            o.publish_status();
            say(format!("Je t’écoute : ce que tu dis est écrit dans {label}. Dis « je valide le prompt » pour l’envoyer."));
            return json!({ "ok": true });
        }
        if action == "reprendre" {
            if self.relay.borrow().is_some() {
                self.stop_relay("bouton");
            }
            let session = target.split(':').next().unwrap_or("").to_string();
            if o.terminal_voice.target_tmux_session().map(|s| s == session).unwrap_or(false) {
                o.terminal_voice.pause();
            }
            self.refresh_cards();
            o.publish_status();
            say("Je reprends la main.".into());
            return json!({ "ok": true });
        }
        json!({ "ok": false })
    }

    /// Card › clic : le terminal de ce volet au premier plan.
    pub async fn show_pane(&self, app: &J) -> J {
        let tm = self.tmux.clone();
        let target = js::gs(app, "target");
        let label = js::gs(app, "label");
        let session = target.split(':').next().filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| tm.session());
        let r = self
            .tool("mcp__system__tmux", json!({ "action": "afficher", "cible": label }), move |_| async move {
                let win = crate::re!(r"\.\d+$").replace(&target, "").into_owned();
                tm.select(&win).await?;
                let _ = tm.run(&["select-pane", "-t", &target], None, 10000).await;
                if !tm.attached(Some(&session)).await {
                    tm.open_terminal(Some(&session));
                } else {
                    let _ = crate::orch::system::call("activer_fenetre", &json!({ "titre": format!("tmux · {session}") })).await;
                }
                Ok(Value::Null)
            })
            .await;
        if js::gb(&r, "ok") {
            json!({ "ok": true, "say": null })
        } else {
            json!({ "ok": false, "say": if js::truthy(&r["say"]) { js::gs(&r, "say") } else { format!("Je n’ai pas pu afficher {}.", js::gs(app, "label")) } })
        }
    }

    /// Card › × : ferme ce volet (et l'agent qui y tourne), avec l'accord oral prévu pour tmux_fermer.
    pub async fn close_pane(&self, app: &J) -> J {
        let tm = self.tmux.clone();
        let target = js::gs(app, "target");
        let t2 = target.clone();
        let r = self.tool("mcp__system__tmux_fermer", json!({ "cible": app["label"] }), move |_| async move { tm.kill_pane(&t2).await.map(|_| Value::Null) }).await;
        if js::gb(&r, "ok") {
            if self.relay.borrow().as_ref().map(|rl| rl.target == target).unwrap_or(false) {
                self.stop_relay("fenetre fermee");
            }
            *self.discovered.borrow_mut() = None;
            self.claude_states.borrow_mut().remove(&target);
            json!({ "ok": true })
        } else {
            json!({ "ok": false, "denied": js::gb(&r, "denied"), "say": r["say"] })
        }
    }

    pub fn start_relay(&self, target: &str, label: &str, agent_pid: u32) {
        let now = js::now_ms();
        *self.relay.borrow_mut() = Some(Relay { target: target.into(), label: label.into(), agent_pid, since: now, last_at: now, sent: 0 });
        let o = self.o.up();
        o.record("tmux_relais_debut", json!({ "cible": label, "volet": target }));
        // Lien vocal : garde ouverte, plus besoin de « Jarvis ».
        o.emit(json!({ "type": "relay_voice", "on": true, "label": label }));
        self.refresh_cards();
    }

    pub fn stop_relay(&self, reason: &str) {
        let Some(r) = self.relay.borrow_mut().take() else { return };
        let o = self.o.up();
        o.record("tmux_relais_fin", json!({ "cible": r.label, "phrases": r.sent, "motif": reason }));
        o.emit(json!({ "type": "relay_voice", "on": false }));
        self.refresh_cards();
    }

    /// Relais actif : true si la phrase a été traitée ici. doubt : voix pas sûre (écrite sans Entrée
    /// à un agent, jamais tapée dans un shell ; les touches dites ne sont pas envoyées).
    pub async fn handle_relay(&self, clean: &str, doubt: bool) -> bool {
        let Some(r) = self.relay.borrow().clone() else { return false };
        if js::now_ms() - r.last_at > RELAY_IDLE_MS {
            self.stop_relay("inactivite");
            return false;
        }
        let t = fold(clean);
        let o = self.o.up();
        let say = |text: String| {
            if !text.is_empty() {
                o.emit(json!({ "type": "say", "text": text }));
            }
        };
        if close_relay_re().is_match(&t) {
            let n = r.sent;
            self.stop_relay("fin");
            say(format!("Je reprends la main. {n} message{} transmis.", if n > 1 { "s" } else { "" }));
            o.publish_status();
            return true;
        }
        // Les réglages d'écoute restent à Jarvis, comme la réponse à une validation en attente.
        if let Some(setting) = o.status.parse(clean) {
            if js::gs(&setting, "kind").contains("listening_mode") {
                return false;
            }
        }
        if !o.pending.borrow().is_empty()
            && !validate_re().is_match(&t)
            && crate::re!(r"^(?:oui|non|vas y|d accord|valide|je valide|confirme|accepte|c est bon|ok|annule|refuse|laisse tomber)\b").is_match(&t)
        {
            return false;
        }
        if let Some(rl) = self.relay.borrow_mut().as_mut() {
            rl.last_at = js::now_ms();
        }
        if let Some(intent) = self.match_(clean, true) {
            o.run_intent(intent, clean).await;
            return true;
        }
        // « Choix 2 » : menu de Claude (touche du chiffre).
        if let Some(c) = crate::re!(r"^(?:choix|option|numero|reponse) (\d|un|deux|trois|quatre)$").captures(&t) {
            if doubt {
                say("Je ne suis pas sûr que ce soit ta voix : rien n’a été choisi.".into());
                return true;
            }
            let k = spoken_number(&c[1]);
            if let Err(e) = self.tmux.send_keys(&r.target, &[&k]).await {
                if e.reason == "no-window" || e.reason == "no-session" {
                    self.stop_relay("fenetre fermee");
                }
                say(self.error_say(&e));
            }
            return true;
        }
        // Claude fermé entre-temps : on n'écrit pas dans le shell resté derrière lui.
        if r.agent_pid != 0 && !process_alive(r.agent_pid) {
            self.stop_relay("agent ferme");
            say(format!("{} s'est fermé. Je reprends la main ; rien n'a été tapé.", r.label));
            o.publish_status();
            return true;
        }
        let agent = r.agent_pid != 0 || crate::re!(r"^Claude\b").is_match(&r.label);
        let dest = json!({ "kind": "tmux", "target": r.target, "label": r.label, "agent": agent });
        let res = o.terminal_voice.dictate(&dest, clean, doubt).await;
        if js::gb(&res, "typed") {
            if let Some(rl) = self.relay.borrow_mut().as_mut() {
                rl.sent += 1;
            }
        }
        o.usage_daily.add_local("local");
        true
    }
}
