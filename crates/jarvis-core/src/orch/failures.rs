// Journal des échecs (orchestrator/failures.js) : toute demande qui n'a pas abouti est gardée dans
// data/echecs.json, consultable dans l'administration (« Échecs ») et confiable à Claude Code pour
// que Jarvis apprenne à réussir la prochaine fois.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use crate::js::{self, J};

const MAX: usize = 300;
pub const STATUSES: &[&str] = &["nouveau", "en_cours", "resolu", "ignore"];

/// Réponse qui signale un échec (« je n'ai pas pu… »).
pub const NEGATIVE_REPLY: &str = r"(?i)\b(?:je n['’]ai pas pu|je n['’]arrive pas|impossible d|ne répond pas|indisponible)\b";

pub fn negative_reply(text: &str) -> bool {
    crate::re!(r"(?i)\b(?:je n['’]ai pas pu|je n['’]arrive pas|impossible d|ne répond pas|indisponible)\b").is_match(text)
}

/// Réponse d'une IA qui a « terminé »… en expliquant qu'elle ne pouvait pas : la phrase en cause.
pub fn incapable(answer: &str) -> Option<String> {
    let head: String = answer.chars().take(400).collect();
    let re = crate::re!(r"(?i)(?:je n['’]ai (?:pas|aucun(?:e)?) (?:les? capacités?|la possibilité|accès|les outils|l['’]outil|les moyens|moyen|les droits|la permission)|je ne (?:peux|pourrai|suis pas en mesure de) (?:pas )?(?:le faire|faire (?:ça|cela|cette)|contrôler|piloter|accéder|dessiner|ouvrir|lancer|exécuter|effectuer|réaliser|agir|manipuler|interagir|cliquer|envoyer|utiliser|modifier|créer|écrire|lire|voir)|je ne suis pas (?:en mesure|capable)|je suis incapable|je ne dispose (?:pas|d['’]aucun)|il m['’]est impossible|ce n['’]est pas possible pour moi|tu (?:devras|vas devoir|dois) le faire (?:toi[- ]même|manuellement)|hors de (?:mes|ma) (?:capacités|portée))");
    if !re.is_match(&head) {
        return None;
    }
    let first = crate::fre!(r"(?<=[.!?])\s").split(&head).next().and_then(|r| r.ok()).unwrap_or(&head).to_string();
    Some(first.chars().take(200).collect())
}

fn done_type(t: &str) -> bool {
    crate::re!(r"^(?:changement_|zone_|reponse_cache$|memoire_ajout$|memoire_oubli$|macro$|dictee_|autorisation_temporaire$|relance_demandee$|action_locale$|action_systeme$|action_google$|session_renouvelee$|tmux_relais_)").is_match(t)
}

/// Issue d'une demande (entrée « demande » n° idx du journal) d'après ce qui la suit :
/// { state: ok | fail | unknown, reason }.
pub fn request_outcome(records: &[J], idx: usize) -> J {
    let start = &records[idx];
    let t0 = js::parse_date_ms(&js::gs(start, "at")).unwrap_or(0);
    let mut tasks: Vec<String> = vec![];
    let (mut ok, mut fail, mut unknown): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    let mut cancelled = false;
    let mut spoke = false;
    let mut i = idx + 1;
    while i < records.len() && i <= idx + 80 {
        let r = &records[i];
        i += 1;
        let d = &r["data"];
        let typ = js::gs(r, "type");
        if typ == "demande" || js::parse_date_ms(&js::gs(r, "at")).unwrap_or(0) - t0 > 30 * 60 * 1000 {
            break;
        }
        let tache = js::gs(d, "tache");
        if !tache.is_empty() && typ != "tache_debut" && !crate::re!(r"^(?:local|cmd|purge)").is_match(&tache) && !tasks.contains(&tache) {
            continue;
        }
        match typ.as_str() {
            "tache_debut" => {
                if !tache.is_empty() {
                    tasks.push(tache)
                }
            }
            "intention_locale" => {
                if negative_reply(&js::gs(d, "reponse")) {
                    fail.get_or_insert(format!("réponse : {}", js::gs(d, "reponse")));
                } else {
                    let t = js::gs(d, "type");
                    ok.get_or_insert(format!("action locale ({})", if t.is_empty() { "sans IA".into() } else { t }));
                }
            }
            "demande_ignoree" => {
                let m = js::gs(d, "motif");
                return json!({ "state": "unknown", "reason": format!("ignorée : {}", if m.is_empty() { "bruit ou fragment".into() } else { m }) });
            }
            "action_locale_erreur" | "action_systeme_erreur" | "action_google_erreur" => {
                let o = js::gs(d, "outil");
                fail.get_or_insert(format!("erreur{} : {}", if o.is_empty() { String::new() } else { format!(" {o}") }, js::gs(d, "erreur")));
            }
            "acces_refuse" => {
                let det = js::gs(d, "detail");
                fail.get_or_insert(format!("refusé par la sécurité{}", if det.is_empty() { String::new() } else { format!(" ({det})") }));
            }
            "validation_refusee" => {
                fail.get_or_insert("validation refusée".into());
            }
            "validation_expiree" => {
                fail.get_or_insert("validation expirée".into());
            }
            "validation_vocale_refusee" => {
                fail.get_or_insert("code dicté à la voix (refusé)".into());
            }
            "tache_annulee" => cancelled = true,
            "clarification_expiree" => {
                unknown.get_or_insert("question de Jarvis restée sans réponse".into());
            }
            "parole" => {
                spoke = true;
                let t = js::gs(d, "texte");
                if negative_reply(&t) || incapable(&t).is_some() {
                    fail.get_or_insert(format!("Jarvis : {t}"));
                }
            }
            _ => {
                if done_type(&typ) {
                    ok.get_or_insert(typ.replace('_', " "));
                }
            }
        }
    }
    for id in &tasks {
        let mut end: Option<&J> = None;
        let mut j = idx + 1;
        while j < records.len() && j <= idx + 800 && end.is_none() {
            let r = &records[j];
            if js::gs(r, "type") == "tache_fin" && js::gs(&r["data"], "tache") == *id && js::gs(&r["data"], "statut") != "quota_epuise" {
                end = Some(r);
            }
            j += 1;
        }
        match end {
            None => {
                unknown.get_or_insert("tâche IA sans fin enregistrée (en cours ou interrompue)".into());
            }
            Some(e) => {
                let d = &e["data"];
                let status = js::gs(d, "statut");
                let cannot = if status == "terminee" { incapable(&js::gs(d, "reponse")) } else { None };
                if let Some(c) = cannot {
                    fail.get_or_insert(format!("l'IA a répondu qu'elle ne pouvait pas : « {c} »"));
                } else if status == "terminee" {
                    let m = js::gs(d, "moteur");
                    ok.get_or_insert(format!("tâche IA terminée ({})", if m.is_empty() { "IA".into() } else { m }));
                } else {
                    let err = js::gs(d, "erreur");
                    fail.get_or_insert(format!("tâche IA en échec{}", if err.is_empty() { String::new() } else { format!(" : {err}") }));
                }
            }
        }
    }
    if let Some(f) = fail {
        return json!({ "state": "fail", "reason": js::slice_chars(&f, 300) });
    }
    if cancelled && ok.is_none() {
        return json!({ "state": "unknown", "reason": "tâche annulée" });
    }
    if let Some(o) = ok {
        return json!({ "state": "ok", "reason": o });
    }
    json!({ "state": "unknown", "reason": unknown.unwrap_or_else(|| if spoke { "Jarvis a répondu, sans trace d’action".into() } else { "aucune suite enregistrée".into() }) })
}

fn fold(text: &str) -> String {
    let t = js::fold_lower(text);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

pub struct Failures {
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    pub items: RefCell<Vec<J>>,
    pub on_change: RefCell<Option<Rc<dyn Fn(&J)>>>,
}

impl Failures {
    pub fn new(file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let items = js::read_json(&file).map(|d| js::arr(&d["items"])).unwrap_or_default();
        Self { file, log, items: RefCell::new(items), on_change: RefCell::new(None) }
    }

    fn save(&self) {
        if let Err(e) = js::write_json(&self.file, &json!({ "items": Value::Array(self.items.borrow().clone()) })) {
            (self.log)(format!("[échecs] enregistrement impossible : {e}"));
        }
    }

    fn changed(&self, item: &J) {
        let cb = self.on_change.borrow().clone();
        if let Some(cb) = cb {
            cb(item);
        }
    }

    /// f : { kind: 'ia' | 'local' | 'action', demande, source, erreur, moteur, intention, outil, tache }.
    /// La même demande qui échoue de nouveau (même motif) incrémente l'échec existant.
    pub fn add(&self, f: J) -> Option<J> {
        let demande = js::slice_chars(js::gs(&f, "demande").trim(), 500);
        let erreur = js::slice_chars(js::gs(&f, "erreur").trim(), 1500);
        if demande.is_empty() && erreur.is_empty() {
            return None;
        }
        let kind = js::gs(&f, "kind");
        let key = format!("{}|{}|{}", kind, fold(&demande), js::slice_chars(&fold(&erreur), 80));
        let now = js::iso_now();
        let item = {
            let mut items = self.items.borrow_mut();
            if let Some(it) = items.iter_mut().find(|i| js::gs(i, "key") == key && js::gs(i, "statut") != "resolu") {
                it["occurrences"] = json!(it["occurrences"].as_i64().unwrap_or(1) + 1);
                it["lastAt"] = json!(now);
                if js::gs(it, "statut") == "ignore" {
                    it["statut"] = json!("nouveau");
                }
                it.clone()
            } else {
                let mut b = [0u8; 4];
                getrandom::getrandom(&mut b).ok();
                let opt = |k: &str| if js::truthy(&f[k]) { f[k].clone() } else { Value::Null };
                let it = json!({
                    "id": hex::encode(b), "key": key, "at": now, "lastAt": now,
                    "kind": if kind.is_empty() { "local".to_string() } else { kind }, "demande": demande,
                    "source": opt("source"), "erreur": erreur, "moteur": opt("moteur"), "intention": opt("intention"),
                    "outil": opt("outil"), "tache": opt("tache"), "occurrences": 1, "statut": "nouveau", "apprentissage": null
                });
                items.insert(0, it.clone());
                if items.len() > MAX {
                    let drop = items.iter().enumerate().rev().find(|(_, i)| {
                        let s = js::gs(i, "statut");
                        s == "resolu" || s == "ignore"
                    });
                    let n = drop.map(|(n, _)| n).unwrap_or(items.len() - 1);
                    items.remove(n);
                }
                it
            }
        };
        self.save();
        self.changed(&item);
        Some(item)
    }

    pub fn list(&self) -> Vec<J> {
        self.items
            .borrow()
            .iter()
            .map(|i| {
                let mut c = i.clone();
                if let Some(o) = c.as_object_mut() {
                    o.remove("key");
                }
                c
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<J> {
        self.items.borrow().iter().find(|i| js::gs(i, "id") == id).cloned()
    }

    pub fn update(&self, id: &str, mut patch: J) -> Option<J> {
        if let Some(st) = patch.get("statut").and_then(|s| s.as_str()) {
            if !STATUSES.contains(&st) {
                patch.as_object_mut().unwrap().remove("statut");
            }
        }
        let item = {
            let mut items = self.items.borrow_mut();
            let it = items.iter_mut().find(|i| js::gs(i, "id") == id)?;
            js::assign(it, patch);
            it.clone()
        };
        self.save();
        self.changed(&item);
        Some(item)
    }

    pub fn remove(&self, id: &str) -> bool {
        let n = self.items.borrow().len();
        self.items.borrow_mut().retain(|i| js::gs(i, "id") != id);
        let changed = self.items.borrow().len() != n;
        if changed {
            self.save();
        }
        changed
    }

    pub fn counts(&self) -> J {
        let items = self.items.borrow();
        let mut c = json!({ "total": items.len() });
        for s in STATUSES {
            c[*s] = json!(items.iter().filter(|i| js::gs(i, "statut") == *s).count());
        }
        c
    }
}

const ENTRY_POINTS: &str = "Points d’entrée : `crates/jarvis-core/src/orch/mod.rs` (submit, run_intent, complete_task), le routeur local `crates/jarvis-core/src/orch/router.rs`, `crates/jarvis-core/src/orch/skill_intents.rs`, `crates/jarvis-core/src/orch/local_skills.rs`, les réglages `crates/jarvis-core/src/orch/status.rs`, les actions Windows `crates/jarvis-core/src/orch/system.rs`, la sécurité `crates/jarvis-core/src/orch/policy.rs` et `crates/jarvis-core/src/orch/security.rs`, tmux (`crates/jarvis-core/src/orch/tmux.rs`, `crates/jarvis-core/src/orch/tmux_voice.rs`), la voix `crates/jarvis-voice/`.";
const CHECK: &str = "4. Vérifie : `cargo build --release` puis `cargo test` à la racine du projet.";

/// Consigne donnée à Claude Code pour apprendre à Jarvis à réussir cette demande.
pub fn learning_prompt(item: &J, root: &str) -> String {
    let kind = js::gs(item, "kind");
    let kind_label = match kind.as_str() {
        "ia" => "tâche confiée à une IA, qui a échoué".to_string(),
        "local" => "commande locale (sans IA) qui n’a pas abouti".to_string(),
        "action" => "action Windows en erreur".to_string(),
        _ => kind.clone(),
    };
    let opt = |k: &str, label: &str| {
        let v = js::gs(item, k);
        if v.is_empty() {
            String::new()
        } else {
            format!("{label}{v}")
        }
    };
    let moteur = js::gs(item, "moteur");
    [
        format!("# Apprendre à Jarvis — échec {}", js::gs(item, "id")),
        String::new(),
        format!("- Demande de l'utilisateur : « {} »", if js::gs(item, "demande").is_empty() { "(inconnue)".into() } else { js::gs(item, "demande") }),
        format!(
            "- Type : {kind_label}{}{}{}",
            if moteur.is_empty() { String::new() } else { format!(" (moteur {moteur})") },
            opt("intention", " · intention "),
            opt("outil", " · outil ")
        ),
        format!(
            "- Origine : {} · {} occurrence(s), dernière le {}",
            if js::gs(item, "source") == "typed" { "clavier" } else { "voix" },
            item["occurrences"].as_i64().unwrap_or(1),
            js::gs(item, "lastAt")
        ),
        format!("- Erreur ou réponse : {}", if js::gs(item, "erreur").is_empty() { "(aucun détail)".into() } else { js::gs(item, "erreur") }),
        String::new(),
        format!("Tu travailles dans le code de Jarvis ({root}). Objectif : que cette demande réussisse la prochaine fois, si possible SANS IA, par une commande locale déterministe."),
        String::new(),
        format!("1. Trouve pourquoi elle a échoué. {ENTRY_POINTS}"),
        "2. Si c’est une commande simple (ouvrir, régler, lancer, chercher…), ajoute-la au routeur local avec ses variantes de formulation en français, et un niveau de sécurité cohérent avec les actions voisines (catalogue de `security.rs`). Une commande simple et autonome peut aussi être une compétence : un fichier dans `skills/` (format dans `skills/README.md`, rechargé à chaud, listé dans l’aide).".into(),
        "3. Sinon, corrige la cause (bug, message d’erreur, délai…).".into(),
        CHECK.into(),
        "5. Ne lis ni ne modifie jamais le dossier `data/` (mémoire, journal scellé, secrets de l’utilisateur).".into(),
        "6. Termine par un court résumé en français : cause, correction, fichiers modifiés.".into(),
        String::new(),
        "Jarvis surveille son code : il affichera « Jarvis a été modifié » et proposera de se relancer.".into(),
    ]
    .join("\n")
}

/// Consigne pour une demande du journal restée sans exécution (administration › Journal).
pub fn request_prompt(p: &J, root: &str) -> String {
    let mut lines = vec![
        format!("# Apprendre à Jarvis — demande n° {} non exécutée", js::s(&p["seq"])),
        String::new(),
        format!("- Demande de l'utilisateur : « {} » ({} le {})", js::gs(p, "demande"), if js::gs(p, "source") == "clavier" { "tapée" } else { "dite" }, js::gs(p, "at")),
        format!("- Ce qui s'est passé : {}", js::gs(p, "reason")),
        String::new(),
        "Suite de cette demande dans le journal de Jarvis :".into(),
        "```".into(),
    ];
    lines.extend(js::str_list(&p["trace"]));
    lines.extend([
        "```".into(),
        String::new(),
        format!("Tu travailles dans le code de Jarvis ({root}). Objectif : trouver une alternative pour que ce TYPE de demande soit exécuté à l'avenir via Jarvis — de préférence sans IA, par une commande locale déterministe (plus rapide, sans jetons), sinon en corrigeant ce qui a échoué."),
        String::new(),
        format!("1. Comprends pourquoi elle n’a pas été exécutée. {ENTRY_POINTS}"),
        "2. Généralise : couvre les formulations voisines en français (et les variantes que Whisper transcrit), pas seulement cette phrase exacte. Une commande simple et autonome peut aussi être une compétence : un fichier dans `skills/` (format dans `skills/README.md`, rechargé à chaud, listé dans l’aide).".into(),
        "3. Garde un niveau de sécurité cohérent avec les actions voisines (catalogue de `security.rs`) : une action destructrice ou irréversible exige un code.".into(),
        CHECK.into(),
        "5. Ne lis ni ne modifie jamais le dossier `data/` (mémoire, journal scellé, secrets de l’utilisateur).".into(),
        "6. Termine par un court résumé en français : cause, alternative mise en place, fichiers modifiés, et la phrase à dire à Jarvis pour l’essayer.".into(),
        String::new(),
        "Jarvis surveille son code : il affichera « Jarvis a été modifié » et proposera de se relancer.".into(),
    ]);
    lines.join("\n")
}

/// Consigne pour un commentaire de l'utilisateur sur une entrée du journal.
pub fn comment_prompt(p: &J, root: &str) -> String {
    let mut lines = vec![
        format!("# Retour de l'utilisateur — entrée n° {} du journal", js::s(&p["seq"])),
        String::new(),
        format!("- Entrée commentée : {} ({}) — {}", js::gs(p, "label"), js::gs(p, "at"), js::gs(p, "summary")),
    ];
    if js::truthy(&p["demande"]) {
        lines.push(format!("- Demande d'origine : « {} »", js::gs(p, "demande")));
    }
    if js::truthy(&p["reason"]) {
        lines.push(format!("- Issue enregistrée par Jarvis : {}", js::gs(p, "reason")));
    }
    lines.push(String::new());
    lines.push("Commentaire de l’utilisateur (c’est lui qui fait foi, même si Jarvis a noté l’action comme réussie) :".into());
    lines.push(String::new());
    lines.extend(js::gs(p, "comment").split('\n').map(|l| format!("> {l}")));
    lines.extend([String::new(), "Contexte dans le journal de Jarvis (→ = entrée commentée) :".into(), "```".into()]);
    lines.extend(js::str_list(&p["trace"]));
    lines.extend([
        "```".into(),
        String::new(),
        format!("Tu travailles dans le code de Jarvis ({root}). Objectif : corriger le comportement décrit dans le commentaire, pour ce TYPE de demande et pas seulement cette phrase exacte."),
        String::new(),
        format!("1. Compare ce que l’utilisateur attendait à ce que Jarvis a fait (journal ci-dessus), et trouve où la décision a été prise. {ENTRY_POINTS} L’interface est dans `ui/`."),
        "2. Corrige la cause. Si l’action a été jugée réussie à tort, rends aussi la vérification du résultat plus juste. Une commande simple et autonome peut aussi être une compétence : un fichier dans `skills/` (format dans `skills/README.md`, rechargé à chaud, listé dans l’aide).".into(),
        "3. Garde un niveau de sécurité cohérent avec les actions voisines (catalogue de `security.rs`) : une action destructrice ou irréversible exige un code.".into(),
        CHECK.into(),
        "5. Ne lis ni ne modifie jamais le dossier `data/` (mémoire, journal scellé, secrets de l’utilisateur).".into(),
        "6. Termine par un court résumé en français : cause, correction, fichiers modifiés, et la phrase à dire à Jarvis pour vérifier.".into(),
        String::new(),
        "Jarvis surveille son code : il affichera « Jarvis a été modifié » et proposera de se relancer.".into(),
    ]);
    lines.join("\n")
}
