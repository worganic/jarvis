// Prononciation (orchestrator/pronunciation.js) : le texte est réécrit juste avant la synthèse vocale
// (l'écran garde l'orthographe) : lexique de l'utilisateur, identifiants découpés
// (« lego-rollerCoaster » → « lego roller coaster »), lexique intégré, élisions oubliées.

use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use crate::js::{self, J};

pub const BUILTIN: &[(&str, &str)] = &[
    ("roller coaster", "rolleur côsteur"),
    ("rollercoaster", "rolleur côsteur"),
    ("coaster", "côsteur"),
    ("tmux", "T mux"),
    ("psmux", "P S mux"),
    ("github", "guite-heub"),
    ("git", "guite"),
    ("npm", "N P M"),
    ("json", "djésonne"),
    ("api", "A P I"),
    ("url", "U R L"),
    ("vs code", "V S code"),
    ("agy", "A G Y"),
    ("gmail", "dji-mèil"),
    ("yeelight", "yi-laïte"),
    ("readme", "rid-mi"),
    ("localhost", "local-host"),
];

const SHORT_WORDS: &[&str] = &["de", "du", "le", "la", "un", "en", "et", "ou", "au", "a", "on", "ma", "mon", "me", "my", "go", "to", "up", "is", "it", "of", "in", "no", "so", "do"];

/// Remplacement mot entier (lettres accentuées comprises), casse ignorée. whole_tokens : un mot
/// relié par - ou _ n'est pris qu'en entier.
fn replace_words(text: &str, dict: &[(String, String)], whole_tokens: bool) -> String {
    let mut keys: Vec<&(String, String)> = dict.iter().filter(|(k, _)| !k.is_empty()).collect();
    if keys.is_empty() {
        return text.to_string();
    }
    keys.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()));
    let w = if whole_tokens { r"\p{L}\p{N}_\-" } else { r"\p{L}\p{N}" };
    let alt = keys.iter().map(|(k, _)| fancy_regex::escape(k).into_owned()).collect::<Vec<_>>().join("|");
    let Ok(re) = fancy_regex::Regex::new(&format!(r"(?i)(^|[^{w}])({alt})(?=$|[^{w}])")) else { return text.to_string() };
    re.replace_all(text, |c: &fancy_regex::Captures| {
        let word = &c[2];
        let rep = keys.iter().find(|(k, _)| k.to_lowercase() == word.to_lowercase()).map(|(_, v)| v.clone()).unwrap_or_else(|| word.to_string());
        format!("{}{}", &c[1], rep)
    })
    .into_owned()
}

/// « lego-rollerCoaster » → « lego roller coaster ».
pub fn split_identifiers(text: &str, names: &HashSet<String>) -> String {
    crate::re!(r"[\p{L}\p{N}]+(?:[-_.][\p{L}\p{N}]+)*")
        .replace_all(text, |c: &regex::Captures| {
            let tok = &c[0];
            let camel = crate::re!(r"\p{Ll}\p{Lu}").is_match(tok);
            let underscore = tok.contains('_');
            let known = names.contains(&tok.to_lowercase());
            let mixed = crate::re!(r"\p{L}\d|\d\p{L}").is_match(tok) && crate::re!(r"[-_]").is_match(tok);
            if !camel && !underscore && !known && !mixed {
                return tok.to_string();
            }
            if crate::re!(r"^\d+(?:[.,]\d+)*$").is_match(tok) || crate::re!(r"(?i)\.[a-z]{2,4}$").is_match(tok) {
                return tok.to_string();
            }
            let t = crate::re!(r"(\p{Ll})(\p{Lu})").replace_all(tok, "$1 $2");
            let t = crate::re!(r"(\p{Lu})(\p{Lu}\p{Ll})").replace_all(&t, "$1 $2");
            let t = crate::re!(r"(\p{L})(\d)").replace_all(&t, "$1 $2");
            let t = crate::re!(r"(\d)(\p{L})").replace_all(&t, "$1 $2");
            crate::re!(r"[\s\-_]+")
                .split(&t)
                .filter(|p| !p.is_empty())
                .map(|p| {
                    if crate::re!(r"^\p{Lu}+$").is_match(p) {
                        return p.to_string();
                    }
                    let low = p.to_lowercase();
                    if low.chars().count() == 2 && crate::re!(r"^\p{L}+$").is_match(&low) && !SHORT_WORDS.contains(&low.as_str()) {
                        low.to_uppercase()
                    } else {
                        low
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .into_owned()
}

/// Élisions oubliées devant une voyelle (« je écoute », « que il »), et « si il ».
pub fn fix_elisions(text: &str) -> String {
    let re = crate::fre!(r#"(?i)(?<=^|[\s«"(—–-])(je|me|te|se|ne|de|que)\s+(?=(\p{L}+))"#);
    let out = re
        .replace_all(text, |c: &fancy_regex::Captures| {
            let word = &c[1];
            let next = &c[2];
            let elidable = crate::re!(r"(?i)^(?:[aeiouéèêëàâîïôûœæ]|y$)").is_match(next);
            let no = crate::re!(r"(?i)^(?:onze|onzième|oui|ouistiti|uhlan|ululer|yaourt|yoga|yacht|yéti|yen|yo)").is_match(next);
            if !elidable || no {
                return c[0].to_string();
            }
            let mut w: Vec<char> = word.chars().collect();
            w.pop();
            format!("{}'", w.into_iter().collect::<String>())
        })
        .into_owned();
    crate::fre!(r#"(?i)(^|[\s«"(])(si)\s+(ils?)(?=$|[^\p{L}])"#)
        .replace_all(&out, |c: &fancy_regex::Captures| format!("{}{}'{}", &c[1], c[2].chars().next().unwrap_or('s'), &c[3]))
        .into_owned()
}

/// Une règle du lexique : `voice` vide = toutes les voix, sinon l'identifiant de la voix (fr-FR-DeniseNeural…).
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub word: String,
    pub say: String,
    pub voice: String,
}

/// Mots de 4 au plus dans une expression du lexique (« Airbus Helicopters »).
pub const MAX_WORDS: usize = 4;
/// Phrases dites gardées pour la card Diction.
const RECENT_MAX: usize = 10;

/// Mots probablement mal lus par une voix française : (suspect, raison). `first` : premier mot d'une phrase.
pub fn suspect(word: &str, first: bool) -> Option<&'static str> {
    let w = word.trim_matches(|c: char| c == '\'' || c == '’' || c == '-');
    let low = w.to_lowercase();
    if w.chars().count() < 2 {
        return None;
    }
    if crate::re!(r"\p{L}").is_match(w) && crate::re!(r"\d").is_match(w) {
        return Some("code ou référence");
    }
    if w.chars().count() >= 2 && w.chars().all(|c| c.is_uppercase() || c.is_ascii_digit()) && w.chars().any(|c| c.is_alphabetic()) {
        return Some("sigle");
    }
    const FRENCH_ERS: &[&str] = &["vers", "envers", "travers", "divers", "univers", "pervers", "revers", "convers", "tiers", "fers", "mers", "nerfs", "clercs"];
    const FRENCH_Y: &[&str] = &["yaourt", "yoga", "yeux", "yacht", "yéti", "yen", "yole", "yoyo"];
    let english = crate::re!(r"(?i)w|k|th|sh|oo|ee|ea[^u]|ck|gh|[^aeiouyéèêiu]y$|^y[aeiou]|[^i]ers$|ing$").is_match(&low)
        && !FRENCH_ERS.contains(&low.as_str())
        && !FRENCH_Y.iter().any(|f| low.starts_with(f))
        && !crate::re!(r"(?i)^(?:kilo|kg|km|ok|week|kiosque|whisky|wagon|show|shampo|chef|steak|sketch|tee|tea|bouquet|képi|klaxon|ketchup|kayak|parking|camping|marketing)").is_match(&low);
    if english {
        return Some("anglais ?");
    }
    if !first && w.chars().next().is_some_and(|c| c.is_uppercase()) {
        return Some("nom propre");
    }
    None
}

pub struct Pronunciation {
    file: PathBuf,
    names: Box<dyn Fn() -> Vec<String>>,
    #[allow(dead_code)]
    log: Rc<dyn Fn(String)>,
    pub entries: RefCell<Vec<Rule>>,
    /// Dernières phrases dites : { id, texte, dit, voix, tache, at }.
    recent: RefCell<VecDeque<J>>,
    seq: Cell<u64>,
}

impl Pronunciation {
    pub fn new(file: PathBuf, names: Box<dyn Fn() -> Vec<String>>, log: Rc<dyn Fn(String)>) -> Self {
        let entries = js::read_json(&file)
            .map(|v| {
                js::arr(&v)
                    .iter()
                    .filter(|e| js::truthy(&e["word"]) && js::truthy(&e["say"]))
                    .map(|e| Rule { word: js::gs(e, "word"), say: js::gs(e, "say"), voice: js::gs(e, "voice") })
                    .collect()
            })
            .unwrap_or_default();
        Self { file, names, log, entries: RefCell::new(entries), recent: RefCell::new(VecDeque::new()), seq: Cell::new(0) }
    }

    fn save(&self) {
        let v: Vec<J> = self.entries.borrow().iter().map(rule_json).collect();
        let _ = js::write_json(&self.file, &Value::Array(v));
    }

    pub fn list(&self) -> Vec<J> {
        self.entries.borrow().iter().map(rule_json).collect()
    }

    /// Règle pour toutes les voix (commande vocale « prononce X comme Y », administration).
    pub fn set(&self, word: &str, say: &str) -> J {
        self.set_for(word, say, "")
    }

    /// Règle pour une voix (`voice` vide : toutes). Un mot ou une expression de MAX_WORDS mots au plus.
    pub fn set_for(&self, word: &str, say: &str, voice: &str) -> J {
        let strip = |s: &str| crate::re!(r#"^[«"“]\s*|\s*[»"”]$"#).replace_all(s.trim(), "").into_owned();
        let w = crate::re!(r"\s+").replace_all(&strip(word), " ").into_owned();
        let s = strip(say);
        let voice = voice.trim().to_string();
        if w.is_empty() || s.is_empty() || w.chars().count() > 60 || s.chars().count() > 80 {
            return json!({ "ok": false, "error": "mot et prononciation requis (60 et 80 caractères au plus)" });
        }
        if w.split(' ').count() > MAX_WORDS {
            return json!({ "ok": false, "error": format!("{MAX_WORDS} mots au plus par expression") });
        }
        if !voice.is_empty() && !crate::re!(r"^[a-z]{2,3}-[A-Z]{2,4}-[A-Za-z]{3,40}Neural$").is_match(&voice) {
            return json!({ "ok": false, "error": "voix inconnue" });
        }
        let key = w.to_lowercase();
        self.entries.borrow_mut().retain(|r| !(r.word.to_lowercase() == key && r.voice == voice));
        self.entries.borrow_mut().push(Rule { word: w.clone(), say: s.clone(), voice: voice.clone() });
        self.save();
        json!({ "ok": true, "word": w, "say": s, "voice": voice })
    }

    /// Retire la règle d'un mot : toutes ses variantes, ou seulement celle d'une voix (`Some("")` : la générale).
    pub fn remove_for(&self, word: &str, voice: Option<&str>) -> J {
        let before = self.entries.borrow().len();
        let key = word.trim().to_lowercase();
        self.entries.borrow_mut().retain(|r| !(r.word.to_lowercase() == key && voice.map(|v| r.voice == v).unwrap_or(true)));
        let changed = self.entries.borrow().len() != before;
        if changed {
            self.save();
        }
        json!({ "ok": changed })
    }

    pub fn remove(&self, word: &str) -> J {
        self.remove_for(word, None)
    }

    /// Règles qui valent pour cette voix, celles de la voix d'abord (elles l'emportent sur les générales).
    fn rules_for(&self, voice: &str) -> Vec<(String, String)> {
        let entries = self.entries.borrow();
        let own: Vec<&Rule> = entries.iter().filter(|r| !voice.is_empty() && r.voice == voice).collect();
        let mut out: Vec<(String, String)> = own.iter().map(|r| (r.word.clone(), r.say.clone())).collect();
        for r in entries.iter().filter(|r| r.voice.is_empty()) {
            if !own.iter().any(|o| o.word.to_lowercase() == r.word.to_lowercase()) {
                out.push((r.word.clone(), r.say.clone()));
            }
        }
        out
    }

    /// Texte envoyé à la voix (règles générales seulement).
    pub fn apply(&self, text: &str) -> String {
        self.apply_for(text, "")
    }

    /// Texte envoyé à la voix `voice` : ses règles, puis les générales, puis le lexique intégré.
    pub fn apply_for(&self, text: &str, voice: &str) -> String {
        if text.trim().is_empty() {
            return text.to_string();
        }
        let user = self.rules_for(voice);
        let builtin: Vec<(String, String)> = BUILTIN.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let names: HashSet<String> = (self.names)().into_iter().map(|n| n.to_lowercase()).collect();
        let mut out = replace_words(text, &user, true);
        out = replace_words(&out, &builtin, true);
        out = split_identifiers(&out, &names);
        out = replace_words(&out, &user, false);
        out = replace_words(&out, &builtin, false);
        fix_elisions(&out)
    }

    // --- Card Diction ---------------------------------------------------------------------------

    /// Phrase dite par Jarvis (texte affiché et texte reçu par la voix). Les morceaux d'une réponse
    /// en flux (même tâche, moins de 30 s) sont réunis en une phrase.
    pub fn remember(&self, text: &str, spoken: &str, voice: &str, task: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let now = js::now_ms();
        let mut recent = self.recent.borrow_mut();
        if let Some(last) = recent.back_mut().filter(|l| !task.is_empty() && js::gs(l, "tache") == task && (now - js::gn_or(l, "at", 0.0) as i64) < 30_000) {
            last["texte"] = json!(js::slice_chars(&format!("{} {text}", js::gs(last, "texte")), 1500));
            last["dit"] = json!(js::slice_chars(&format!("{} {spoken}", js::gs(last, "dit")), 1500));
            last["at"] = json!(now);
            return;
        }
        self.seq.set(self.seq.get() + 1);
        recent.push_back(json!({ "id": self.seq.get(), "texte": js::slice_chars(text, 1500), "dit": js::slice_chars(spoken, 1500), "voix": voice, "tache": task, "at": now }));
        while recent.len() > RECENT_MAX {
            recent.pop_front();
        }
    }

    /// Les `n` dernières phrases, la plus récente en premier, découpées en mots pour la card.
    pub fn recent_view(&self, n: usize, voice: &str) -> Vec<J> {
        self.recent
            .borrow()
            .iter()
            .rev()
            .take(n)
            .map(|p| {
                let mut v = p.clone();
                v["mots"] = json!(self.tokens(&js::gs(p, "texte"), voice));
                v
            })
            .collect()
    }

    /// Mots d'une phrase : position, lecture par la voix, règle en place, suspect ou non.
    pub fn tokens(&self, text: &str, voice: &str) -> Vec<J> {
        let rules = self.rules_for(voice);
        let lower = text.to_lowercase();
        // Expressions du lexique présentes dans la phrase : leurs mots sont marqués comme corrigés.
        let mut covered: Vec<(usize, usize, String, String)> = vec![];
        for (w, s) in rules.iter().filter(|(w, _)| w.contains(' ')) {
            let pat = format!(r"(?i)(?:^|[^\p{{L}}\p{{N}}])({})(?:$|[^\p{{L}}\p{{N}}])", regex::escape(&w.to_lowercase()));
            if let Ok(re) = regex::Regex::new(&pat) {
                for c in re.captures_iter(&lower) {
                    let m = c.get(1).unwrap();
                    covered.push((m.start(), m.end(), w.clone(), s.clone()));
                }
            }
        }
        let mut out = vec![];
        let mut first = true;
        let mut last_end = 0;
        for m in crate::re!(r"[\p{L}\p{N}]+(?:['’\-][\p{L}\p{N}]+)*").find_iter(text) {
            let between = &text[last_end..m.start()];
            if crate::re!(r"[.!?…:]").is_match(between) {
                first = true;
            }
            last_end = m.end();
            let word = m.as_str();
            let expr = covered.iter().find(|(s, e, _, _)| m.start() >= *s && m.end() <= *e);
            let own = rules.iter().find(|(w, _)| w.to_lowercase() == word.to_lowercase());
            let (rule, said) = match (expr, own) {
                (Some((_, _, w, s)), _) => (Some(json!({ "mot": w, "dire": s, "expression": true })), s.clone()),
                (None, Some((w, s))) => (Some(json!({ "mot": w, "dire": s })), s.clone()),
                (None, None) => (None, self.apply_for(word, voice)),
            };
            let auto = rule.is_none() && said != word;
            let why = if rule.is_some() { None } else { suspect(word, first) };
            out.push(json!({
                "mot": word, "debut": text[..m.start()].chars().count(), "fin": text[..m.end()].chars().count(),
                "dit": said, "regle": rule, "auto": auto, "suspect": why.is_some(), "raison": why,
            }));
            first = false;
        }
        out
    }
}

/// Contenu de la card Diction : les 3 dernières phrases découpées en mots, et la voix en cours.
pub fn diction_view(o: &super::Orchestrator) -> J {
    let (voice, name) = {
        let d = o.status.d.borrow();
        (d.voice.clone(), d.voice_name.clone())
    };
    json!({ "ok": true, "phrases": o.pronunciation.recent_view(3, &voice), "voix": voice, "voixNom": name, "presentation": o.savoir.state() })
}

/// Opérations de la card Diction (méthode « diction » du HUD).
pub fn diction_op(o: &super::Orchestrator, op: &str, a: &J) -> J {
    let say = |text: &str| {
        let t = js::slice_chars(text.trim(), 600);
        if !t.is_empty() {
            o.emit(json!({ "type": "say", "text": t, "diction": true })); // essai : pas gardé comme phrase de Jarvis
        }
    };
    match op {
        "ouvrir" => {
            let v = diction_view(o);
            o.show_widget("diction_view", v.clone());
            v
        }
        "recentes" => diction_view(o),
        // Présentation Data en cours : pause, reprise (la scène est redite), scène précédente ou suivante, début, arrêt.
        "pilote" => {
            let c = js::gs(a, "action");
            if !["pause", "reprendre", "precedent", "suivant", "debut", "stop"].contains(&c.as_str()) || !o.savoir.control(&c) {
                return json!({ "ok": false, "error": "aucune présentation en cours" });
            }
            diction_view(o)
        }
        // ▶ tel qu'il est dit (lexique actuel), ou ▶ la proposition : le texte passe par la voix comme une phrase.
        "ecouter" | "essayer" => {
            say(&js::gs(a, if op == "ecouter" { "texte" } else { "dire" }));
            json!({ "ok": true })
        }
        "enregistrer" => {
            let voice = if js::gb(a, "toutesVoix") { String::new() } else { o.status.d.borrow().voice.clone() };
            let r = o.pronunciation.set_for(&js::gs(a, "mot"), &js::gs(a, "dire"), &voice);
            if !js::gb(&r, "ok") {
                return r;
            }
            o.record("reglages_modifies", json!({ "prononciation": format!("{} → {}", js::gs(&r, "word"), js::gs(&r, "say")), "voix": if voice.is_empty() { "toutes".to_string() } else { voice }, "origine": "diction" }));
            let mut v = diction_view(o);
            v["enregistre"] = r;
            v
        }
        "oublier" => {
            o.pronunciation.remove_for(&js::gs(a, "mot"), None); // la règle du mot, pour toutes les voix
            o.record("reglages_modifies", json!({ "prononciation": format!("{} oublié", js::gs(a, "mot")), "origine": "diction" }));
            diction_view(o)
        }
        _ => json!({ "ok": false, "error": "opération inconnue" }),
    }
}

fn rule_json(r: &Rule) -> J {
    json!({ "word": r.word, "say": r.say, "voice": r.voice })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lexicon(rules: &[(&str, &str, &str)]) -> Pronunciation {
        let file = std::env::temp_dir().join(format!("prononciation-test-{}-{}.json", std::process::id(), rules.len()));
        let _ = std::fs::remove_file(&file);
        let p = Pronunciation::new(file, Box::new(Vec::new), Rc::new(|_| {}));
        for (w, s, v) in rules {
            assert!(js::gb(&p.set_for(w, s, v), "ok"), "{w}");
        }
        p
    }

    #[test]
    fn mots_suspects() {
        for w in ["Helicopters", "H160", "GitHub", "SNCF", "Bluetooth", "meeting", "Wednesday"] {
            assert!(suspect(w, true).is_some(), "{w} devrait être suspect");
        }
        for w in ["maison", "réunion", "volontiers", "univers", "travers", "kilomètre", "yaourt", "premiers", "dents", "alors", "sports", "Bonjour"] {
            assert!(suspect(w, true).is_none(), "{w} ne devrait pas l'être : {:?}", suspect(w, true));
        }
        assert_eq!(suspect("Marseille", false), Some("nom propre"));
        assert_eq!(suspect("Marseille", true), None, "majuscule de début de phrase");
    }

    #[test]
    fn regle_de_la_voix_avant_la_generale() {
        let p = lexicon(&[("helicopters", "hélicoptère", ""), ("helicopters", "hélicoptèrse", "fr-FR-HenriNeural")]);
        assert_eq!(p.apply_for("Airbus Helicopters", "fr-FR-DeniseNeural"), "Airbus hélicoptère");
        assert_eq!(p.apply_for("Airbus Helicopters", "fr-FR-HenriNeural"), "Airbus hélicoptèrse");
        p.remove_for("helicopters", Some("fr-FR-HenriNeural"));
        assert_eq!(p.apply_for("Airbus Helicopters", "fr-FR-HenriNeural"), "Airbus hélicoptère", "la générale reste");
    }

    #[test]
    fn expression_avant_mot_isole() {
        let p = lexicon(&[("helicopters", "hélicoptère", ""), ("airbus helicopters", "èrbusse hélicoptère", "")]);
        assert_eq!(p.apply("Airbus Helicopters et des helicopters"), "èrbusse hélicoptère et des hélicoptère");
        let t = p.tokens("Airbus Helicopters et des helicopters", "");
        assert_eq!(t[0]["regle"]["expression"], json!(true));
        assert_eq!(t[1]["regle"]["expression"], json!(true));
        assert!(t[4]["regle"]["expression"].is_null() && t[4]["dit"] == json!("hélicoptère"));
        assert!(!js::gb(&p.set_for("un deux trois quatre cinq", "x", ""), "ok"), "5 mots refusés");
        assert!(!js::gb(&p.set_for("mot", "x", "pas une voix"), "ok"));
    }

    #[test]
    fn decoupage_en_mots() {
        let p = lexicon(&[]);
        let t = p.tokens("Bonjour, l'équipe d'Airbus Helicopters ! Le H160 vole.", "");
        let mots: Vec<String> = t.iter().map(|m| js::gs(m, "mot")).collect();
        assert_eq!(mots, vec!["Bonjour", "l'équipe", "d'Airbus", "Helicopters", "Le", "H160", "vole"]);
        assert_eq!((t[3]["debut"].as_u64(), t[3]["fin"].as_u64()), (Some(27), Some(38)));
        assert_eq!(t[3]["raison"], json!("anglais ?"));
        assert!(t[4]["suspect"] == json!(false), "« Le » après « ! » : début de phrase");
        assert_eq!(t[5]["raison"], json!("code ou référence"));
    }

    #[test]
    fn ancien_format_lu_et_phrases_reunies() {
        let file = std::env::temp_dir().join(format!("prononciation-ancien-{}.json", std::process::id()));
        std::fs::write(&file, r#"[{"word":"coaster","say":"côsteur"}]"#).unwrap();
        let p = Pronunciation::new(file.clone(), Box::new(Vec::new), Rc::new(|_| {}));
        assert_eq!(p.list(), vec![json!({ "word": "coaster", "say": "côsteur", "voice": "" })]);
        p.remember("Je cherche", "Je cherche", "", "t1");
        p.remember("les infos.", "les infos.", "", "t1");
        p.remember("Autre chose.", "Autre chose.", "", "");
        let r = p.recent_view(3, "");
        assert_eq!(r.len(), 2);
        assert_eq!(r[1]["texte"], json!("Je cherche les infos."), "morceaux d'une même réponse réunis");
        let _ = std::fs::remove_file(&file);
    }
}
