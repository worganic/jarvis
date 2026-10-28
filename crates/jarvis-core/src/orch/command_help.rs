// Aide aux commandes (orchestrator/command-help.js) : « je ne sais plus quelle commande dire pour lancer
// Claude Code dans tmux sous le projet Shelley » → la phrase exacte à dire, et Jarvis propose de
// l'exécuter (« oui » dans les 45 s). Tout est local : un catalogue (data/command-catalog.json, exporté
// de la v3) avec des mots-clés, et un score par mots en commun pondérés par leur rareté.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use serde_json::{json, Value};

use super::intent::Intent;
use super::{OrchRef, Up};
use crate::js::{self, J};

static CATALOG: LazyLock<Vec<J>> = LazyLock::new(|| serde_json::from_str(include_str!("../../data/command-catalog.json")).unwrap_or_default());

const STOP_WORDS: &str = "avoir obtenir je tu il on nous vous me te le la les l un une des de du d a au aux et ou en y ne pas plus sais savoir quelle quel quels quelles est c ce que qui quoi dois doit dire faut faire fais comment pour sur sous dans avec mon ma mes ton ta tes son sa ses moi toi aide aider help commande commandes rappelle rappeler jarvis stp s il plait peux puis veux voudrais aimerais besoin oublie oublier deja encore alors bon bien donc svp est ce qu";
static STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| STOP_WORDS.split(' ').collect());

fn spoken_blank(k: &str) -> String {
    match k {
        "projet" => "tel projet",
        "ville" => "telle ville",
        "app" => "telle application",
        "fichier" => "tel fichier",
        "texte" => "ce que tu veux",
        "nom" => "telle phrase",
        "qui" => "telle personne",
        "jour" => "tel jour",
        "commande" => "telle commande",
        "site" => "tel site",
        other => other,
    }
    .to_string()
}

fn cat_icon(c: &str) -> &'static str {
    match c {
        "Conversation" => "◉",
        "Projets" => "◧",
        "tmux & Claude Code" => "◈",
        "Musique" => "♫",
        "Mails & agenda" => "✉",
        "Informations" => "ℹ",
        "Applications & fichiers" => "▤",
        "Système" => "⚙",
        "Raccourcis & mémoire" => "✦",
        "Jarvis" => "⬡",
        "Mes compétences" => "✚",
        "Rappels" => "◷",
        "Maison" => "☀",
        "Achats" => "🛒",
        _ => "▸",
    }
}

fn has_blank(s: &str) -> bool {
    crate::re!(r"\{[a-z]+\}").is_match(s)
}

fn bracketed(s: &str) -> String {
    crate::re!(r"\{([a-z]+)\}").replace_all(s, "[$1]").into_owned()
}

/// Ligne de card : cliquable si la phrase est complète et valable hors contexte.
fn help_row(e: &J) -> J {
    let say = js::gs(e, "say");
    let complete = !has_blank(&say) && !js::truthy(&e["context"]);
    let mut r = json!({ "icon": cat_icon(&js::gs(e, "cat")), "title": format!("« {} »", bracketed(&say)), "sub": e["what"] });
    if complete {
        r["say"] = json!(say);
    }
    r
}

pub fn fold(text: &str) -> String {
    let t = js::fold_lower(text).replace(['\'', '’'], " ");
    let t = crate::re!(r"[^a-z0-9 ]+").replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

/// Racine grossière : « lancer / lance / lancement » → « lanc ».
fn stem(w: &str) -> String {
    if w.chars().count() > 5 {
        w.chars().take(5).collect()
    } else {
        w.strip_suffix('s').unwrap_or(w).to_string()
    }
}

fn words_of(e: &J) -> HashSet<String> {
    let say = crate::re!(r"\{[a-z]+\}").replace_all(&js::gs(e, "say"), " ").into_owned();
    fold(&format!("{} {say} {}", js::gs(e, "keys"), js::gs(e, "what"))).split(' ').filter(|w| !w.is_empty() && !STOP.contains(w)).map(stem).collect()
}

static INDEX: LazyLock<Vec<(J, HashSet<String>)>> = LazyLock::new(|| CATALOG.iter().map(|e| (e.clone(), words_of(e))).collect());
static DF: LazyLock<HashMap<String, usize>> = LazyLock::new(|| {
    let mut df = HashMap::new();
    for (_, w) in INDEX.iter() {
        for x in w {
            *df.entry(x.clone()).or_insert(0) += 1;
        }
    }
    df
});

fn weight(w: &str) -> f64 {
    1.0 / (DF.get(w).copied().unwrap_or(1).max(1) as f64).sqrt()
}

pub fn catalog() -> &'static [J] {
    &CATALOG
}

pub struct CommandHelp {
    o: OrchRef,
    pub suggestion: RefCell<Option<(String, i64)>>,
}

pub struct SearchResult {
    pub project: Option<J>,
    pub words: usize,
    pub results: Vec<(J, f64, usize)>,
}

impl CommandHelp {
    pub fn new(o: OrchRef) -> Self {
        Self { o, suggestion: RefCell::new(None) }
    }

    /// Projet cité dans la question : son nom exact remplace {projet}.
    fn project_in(&self, q: &str) -> Option<J> {
        let o = self.o.up();
        if let Some(m) = crate::re!(r"\bprojets? ([a-z0-9 ]+)$").captures(q) {
            if let Some(p) = o.projects.find(&m[1]) {
                if js::gn_or(&p, "score", 0.0) >= 60.0 {
                    return Some(p);
                }
            }
        }
        let mut best: Option<J> = None;
        for p in o.projects.list() {
            let f = js::gs(&p, "folded");
            if f.chars().count() >= 3 && regex::Regex::new(&format!(r"\b{}\b", regex::escape(&f))).map(|r| r.is_match(q)).unwrap_or(false) && best.as_ref().map(|b| f.len() > js::gs(b, "folded").len()).unwrap_or(true) {
                best = Some(p);
            }
        }
        best
    }

    /// Commandes ajoutées par l'utilisateur (skills/*.js), au même format que le catalogue.
    fn extra(&self) -> Vec<J> {
        self.o.up().skills.help_entries()
    }

    /// Meilleures commandes pour une question.
    pub fn search(&self, question: &str, limit: usize) -> SearchResult {
        let q = fold(question);
        let tokens: Vec<&str> = q.split(' ').filter(|w| !w.is_empty() && !STOP.contains(w)).collect();
        let project = self.project_in(&q);
        let pw: HashSet<String> = project.as_ref().map(|p| js::gs(p, "folded").split(' ').map(String::from).collect()).unwrap_or_default();
        let words: Vec<String> = tokens.into_iter().filter(|w| !pw.contains(*w)).map(stem).collect();
        let tmux_asked = crate::re!(r"\b(?:tmux|psmux)\b").is_match(&q);
        let extra: Vec<(J, HashSet<String>)> = self.extra().into_iter().map(|e| {
            let w = words_of(&e);
            (e, w)
        }).collect();
        let mut results: Vec<(J, f64, usize)> = vec![];
        for (e, ew) in INDEX.iter().chain(extra.iter()) {
            let mut score = 0.0;
            let mut hits = 0;
            for w in &words {
                if ew.contains(w) {
                    hits += 1;
                    score += 1.0 + weight(w);
                }
            }
            if hits == 0 {
                continue;
            }
            score += hits as f64 / ew.len().max(1) as f64;
            if project.is_some() && js::gs(e, "say").contains("{projet}") {
                score += 1.5;
            }
            if js::gs(e, "cat").starts_with("tmux") {
                score += if tmux_asked { 1.0 } else { -0.5 };
            } else if tmux_asked {
                score -= 0.5;
            }
            results.push((e.clone(), score, hits));
        }
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let pname = project.as_ref().map(|p| js::gs(p, "name"));
        let results = results
            .into_iter()
            .take(limit)
            .map(|(mut e, s, h)| {
                if let Some(n) = &pname {
                    e["say"] = json!(js::gs(&e, "say").replacen("{projet}", n, 1));
                }
                (e, s, h)
            })
            .collect();
        SearchResult { project, words: words.len(), results }
    }

    /// Toutes les commandes du catalogue, compétences apprises comprises (page d'aide complète).
    pub fn all(&self) -> Vec<J> {
        CATALOG.iter().cloned().chain(self.extra()).collect()
    }

    pub fn overview(&self) -> (Vec<String>, Vec<String>, Vec<J>) {
        let all: Vec<J> = CATALOG.iter().cloned().chain(self.extra()).collect();
        let mut cats: Vec<String> = vec![];
        for e in &all {
            let c = js::gs(e, "cat");
            if !cats.contains(&c) {
                cats.push(c);
            }
        }
        let mut lines = vec![];
        let mut rows = vec![];
        for c in &cats {
            lines.push(format!("— {}", c.to_uppercase()));
            rows.push(json!({ "section": c }));
            for e in all.iter().filter(|x| js::gs(x, "cat") == *c) {
                lines.push(format!("« {} » : {}", bracketed(&js::gs(e, "say")), js::gs(e, "what")));
                rows.push(help_row(e));
            }
        }
        (cats, lines, rows)
    }

    fn show(o: &super::Orchestrator, title: &str, rows: Vec<J>, footer: &str) {
        let count = rows.iter().filter(|r| !js::truthy(&r["section"])).count();
        o.show_widget("tool", json!({ "title": title, "tool": "aide aux commandes", "count": count, "rows": rows, "footer": footer }));
    }

    /// Intention d'aide pour une phrase, ou None (ce n'est pas une demande d'aide).
    pub fn match_(&self, text: &str) -> Option<Intent> {
        let t = crate::re!(r"^jarvis ").replace(&fold(text), "").into_owned();
        let oc = self.o.clone();
        if crate::re!(r"^(?:aide|aide moi|help|a l aide|j ai besoin d aide|quelles sont (?:tes|les) commandes|liste (?:des|de tes|tes) commandes|que (?:sais|peux) tu faire|qu est ce que tu (?:sais|peux) faire|tes commandes|commandes)$").is_match(&t) {
            return Some(Intent::new("help_overview", "Aide", "commandes disponibles", move || {
                let o = oc.clone();
                async move {
                    let orch = o.up();
                    let (cats, _, rows) = orch.command_help.overview();
                    Self::show(&orch, "Ce que tu peux me dire", rows, "clique sur une commande pour la lancer");
                    let lower: Vec<String> = cats.iter().map(|c| c.to_lowercase()).collect();
                    let (last, head) = lower.split_last().map(|(l, h)| (l.clone(), h.join(", "))).unwrap_or_default();
                    json!({
                        "say": format!("Je peux t'aider pour {head} et {last}. La liste est à l'écran. Tu peux aussi me décrire ce que tu veux faire : par exemple, « aide pour lancer Claude Code dans un projet »."),
                        "display": format!("{} commandes", CATALOG.len())
                    })
                }
            }));
        }
        let strong = crate::re!(r"\b(?:aide|aides|aider|help|commande|commandes|quoi dire|que dire|ce que je (?:dois|peux) dire|je dois dire|je peux dire|faut il dire|dois je dire|que (?:sais|peux) tu faire|qu est ce que tu (?:sais|peux) faire)\b").is_match(&t);
        let weak = crate::re!(r"^(?:comment (?:je fais|faire|on fait|dois je faire|je peux|puis je|est ce que je peux|je lance|je demarre|j ouvre)|rappelle moi comment)\b").is_match(&t);
        if !strong && !weak {
            return None;
        }
        let sr = self.search(&t, 4);
        let best = sr.results.first().cloned();
        if !strong {
            let ok = best.as_ref().map(|(_, _, hits)| *hits >= 2 || (*hits >= 1 && sr.words <= 2)).unwrap_or(false);
            if !ok {
                return None;
            }
        }
        let detail = best.as_ref().map(|b| js::gs(&b.0, "say")).unwrap_or_else(|| "aucune commande".into());
        let results: Vec<J> = sr.results.iter().map(|r| r.0.clone()).collect();
        Some(Intent::new("help_search", "Aide", &detail, move || {
            let (o, best, results) = (oc.clone(), best.clone(), results.clone());
            async move {
                let orch = o.up();
                let Some((best, _, _)) = best else {
                    let (_, _, rows) = orch.command_help.overview();
                    Self::show(&orch, "Ce que tu peux me dire", rows, "clique sur une commande pour la lancer");
                    return json!({ "say": "Je n'ai pas de commande pour ça. La liste de ce que je sais faire est à l'écran ; sinon, demande-le-moi simplement, je chercherai.", "display": "Aucune commande" });
                };
                let say = js::gs(&best, "say");
                Self::show(&orch, "Commandes pour ta demande", results.iter().map(help_row).collect(), "clique sur une commande pour la lancer");
                let context = js::gs(&best, "context");
                let ready = !has_blank(&say) && context != "relais";
                if ready {
                    *orch.command_help.suggestion.borrow_mut() = Some((say.clone(), js::now_ms()));
                }
                let others = if results.len() > 1 { " Les autres possibilités sont à l'écran." } else { "" };
                let note = if context == "relais" { " Ça marche pendant le relais vers Claude." } else { "" };
                let spoken = crate::re!(r"\{([a-z]+)\}").replace_all(&say, |c: &regex::Captures| spoken_blank(&c[1])).into_owned();
                let text = if ready { format!("Dis : « {say} ».{others} Tu veux que je le fasse maintenant ?") } else { format!("Dis : « {spoken} ».{note}{others}") };
                json!({ "say": text, "display": bracketed(&say) })
            }
        }))
    }

    /// « Oui », « vas-y » juste après une aide : la commande proposée est exécutée.
    pub fn take_suggestion(&self, text: &str) -> Option<String> {
        let (s, at) = self.suggestion.borrow().clone()?;
        if js::now_ms() - at > 45000 {
            return None;
        }
        if !crate::re!(r"^(?:oui|ouais|vas y|ok|d accord|fais le|lance la|lance le|execute|go|c est parti|oui vas y|oui fais le)$").is_match(&fold(text)) {
            return None;
        }
        *self.suggestion.borrow_mut() = None;
        Some(s)
    }
}

#[allow(unused)]
fn _v(_: Value) {}
