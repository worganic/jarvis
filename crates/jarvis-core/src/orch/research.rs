// Recherche web avec sources (orchestrator/research.js) : « fais-moi une recherche sur les batteries au
// sodium ». Claude Code part en tâche de fond (WebSearch / WebFetch), écrit un rapport Markdown sourcé
// dans workspace/recherches/, puis Jarvis annonce l'essentiel et affiche une card cliquable.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::json;

use super::intent::Intent;
use super::{OrchRef, Up};
use crate::js::{self, J};

pub fn research_re() -> &'static regex::Regex {
    crate::re!(r"^(?:(?:fais|lance|mene|realise)(?: moi)? )?(?:une |des )?(?:recherche|recherches|enquete|etude|synthese|veille)(?: approfondie| complete| detaillee| web| documentee)? (?:sur|a propos de|a propos des|concernant|au sujet de|au sujet des) (.+)$|^(?:renseigne toi|documente toi|informe toi|cherche des infos|trouve des infos|trouve moi des infos|cherche moi des infos) (?:sur|a propos de|a propos des|concernant) (.+)$")
}

/// Sujet tel qu'il a été dit (casse et accents d'origine).
pub fn topic_from(raw: &str, folded: &str) -> String {
    match crate::re!(r"(?i)(?:sur|à propos de|à propos des|a propos de|concernant|au sujet de|au sujet des)\s+(.+?)[\s.!?]*$").captures(raw) {
        Some(m) => m[1].trim().to_string(),
        None => folded.trim().to_string(),
    }
}

pub fn slug(text: &str) -> String {
    let t = js::fold_lower(if text.is_empty() { "recherche" } else { text });
    let t = crate::re!(r"[^a-z0-9]+").replace_all(&t, "-");
    let t = crate::re!(r"^-|-$").replace_all(&t, "");
    let t = js::slice_chars(&t, 50);
    if t.is_empty() { "recherche".into() } else { t }
}

/// Sources « - [titre](url) » du rapport.
pub fn sources_of(markdown: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for m in crate::re!(r"\[([^\]]{2,200})\]\((https?://[^\s)]+)\)").captures_iter(markdown) {
        if out.len() >= 12 {
            break;
        }
        if !out.iter().any(|(_, u)| u == &m[2]) {
            out.push((m[1].trim().to_string(), m[2].to_string()));
        }
    }
    out
}

pub struct Research {
    o: OrchRef,
    /// consigne envoyée à Claude → (sujet, fichier)
    pending: RefCell<HashMap<String, (String, PathBuf)>>,
}

impl Research {
    pub fn new(o: OrchRef) -> Self {
        Self { o, pending: RefCell::new(HashMap::new()) }
    }

    fn dir(&self) -> PathBuf {
        crate::paths::workspace().join("recherches")
    }

    fn prompt(topic: &str, file: &str) -> String {
        [
            format!("Recherche approfondie sur « {topic} »."),
            "Utilise la recherche web (WebSearch puis WebFetch) : au moins 4 sources fiables et récentes, croisées entre elles.".into(),
            format!("Écris un rapport en français, en Markdown, dans le fichier {file} (crée le dossier au besoin) :"),
            "un titre, « L’essentiel » en 5 lignes, les points clés avec des chiffres quand il y en a, les limites ou désaccords entre sources,".into(),
            "et une section « Sources » avec une ligne par source au format « - [titre](url) ».".into(),
            "Ta réponse finale est lue à voix haute : trois phrases au plus, sans markdown, l’essentiel à retenir.".into(),
        ]
        .join(" ")
    }

    /// Intention du routeur, ou None.
    pub fn match_(&self, t: &str, raw: &str) -> Option<Intent> {
        let m = research_re().captures(t)?;
        if crate::re!(r"\bsur (?:google(?: maps)?|youtube|wikipedia|amazon|maps|images|leboncoin)$").is_match(t) {
            return None;
        }
        let folded = m.get(1).or(m.get(2)).map(|x| x.as_str()).unwrap_or("");
        let topic = topic_from(raw, folded);
        if topic.chars().count() < 3 {
            return None;
        }
        let o = self.o.clone();
        let tp = topic.clone();
        Some(Intent::new("research", "Recherche web", &topic, move || {
            let o = o.clone();
            let topic = tp.clone();
            async move {
                let orch = o.up();
                let me = &orch.research;
                let stamp = chrono::Utc::now().format("%Y%m%d-%H%M").to_string();
                let file = me.dir().join(format!("{}-{stamp}.md", slug(&topic)));
                let text = Self::prompt(&topic, &file.to_string_lossy());
                me.pending.borrow_mut().insert(text.clone(), (topic.clone(), file));
                orch.enqueue_ai(text, json!({ "background": true }));
                json!({ "say": format!("Je lance la recherche sur {topic}. Je te préviens quand le rapport est prêt."), "display": format!("Recherche : {topic}"), "quiet": true })
            }
        }))
    }

    /// Fin de la tâche de fond : true si c'était une recherche (annonce faite ici).
    pub fn done(&self, text: &str, answer: &str, ok: bool) -> bool {
        let Some((topic, file)) = self.pending.borrow_mut().remove(text) else { return false };
        let o = self.o.up();
        if !ok {
            o.notify(json!({ "text": format!("La recherche sur {topic} n'a pas abouti."), "source": "recherche" }));
            return true;
        }
        let md = std::fs::read_to_string(&file).unwrap_or_default();
        let sources = sources_of(if md.is_empty() { answer } else { &md });
        let spoken = crate::re!(r"[#*_`>]").replace_all(answer, "");
        let spoken = js::slice_chars(crate::re!(r"\s+").replace_all(&spoken, " ").trim(), 600);
        let fpath = file.to_string_lossy().to_string();
        let mut rows: Vec<J> = vec![];
        if !md.is_empty() {
            rows.push(json!({ "icon": "▤", "title": "Ouvrir le rapport", "sub": file.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(), "say": format!("Ouvre {fpath}") }));
        }
        if !sources.is_empty() {
            rows.push(json!({ "section": format!("Sources ({})", sources.len()) }));
        }
        for (title, url) in &sources {
            let host = crate::re!(r"^https?://(?:www\.)?").replace(url, "");
            let host = host.split('/').next().unwrap_or("").to_string();
            rows.push(json!({ "icon": "↗", "title": title, "sub": host, "say": format!("Ouvre {url}") }));
        }
        o.notify(json!({
            "text": format!("Recherche terminée sur {topic}. {spoken}"),
            "title": format!("Recherche · {topic}"),
            "source": "recherche",
            "rows": rows,
            "footer": if md.is_empty() { "le rapport n’a pas été écrit : réponse seule".to_string() } else { fpath }
        }));
        true
    }
}
