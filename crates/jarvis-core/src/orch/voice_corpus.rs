// Corpus vocal de l'utilisateur (orchestrator/voice-corpus.js) : phrases à lire à voix haute pour mesurer
// la reconnaissance et calibrer le filtre du locuteur. Les enregistrements restent sur ce PC
// (data/voice-corpus/), jamais transmis. Les nombres sont écrits en chiffres, comme Whisper les transcrit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::js::{self, J};

/// Version de l'extracteur d'empreinte (jarvis-voice › speaker › EMBEDDER_VERSION).
pub const SPEAKER_EMBEDDER: &str = "wespeaker-fbank-cmn-1-rs";

pub const PHRASES: &[(&str, &str, &str)] = &[
    // Commandes du quotidien (dont les impératifs souvent mal transcrits)
    ("c01", "Jarvis, mets de la musique sur Spotify.", "commande"),
    ("c02", "Jarvis, lis le dernier mail que j’ai reçu.", "commande"),
    ("c03", "Jarvis, quelle heure est-il ?", "commande"),
    ("c04", "Mets la musique en pause.", "commande"),
    ("c05", "Passe à la chanson suivante.", "commande"),
    ("c06", "Baisse le son, s’il te plaît.", "commande"),
    ("c07", "Monte le volume à 60 %.", "commande"),
    ("c08", "Lance la playlist Angus et Julia Stone.", "commande"),
    ("c09", "Ouvre Discord.", "commande"),
    ("c10", "Lance Visual Studio Code.", "commande"),
    ("c11", "Qu’est-ce que j’ai demain dans mon agenda ?", "commande"),
    ("c12", "Ajoute un rendez-vous chez le dentiste jeudi à 14h30.", "commande"),
    ("c13", "Cherche les mails d’Amazon.", "commande"),
    ("c14", "Trouve le fichier devis.", "commande"),
    ("c15", "Ouvre mes téléchargements.", "commande"),
    ("c16", "Quel temps fera-t-il demain à Lyon ?", "commande"),
    ("c17", "Combien font 15 % de 240 ?", "commande"),
    ("c18", "Crée un Google Doc.", "commande"),
    ("c19", "Fais une capture d’écran.", "commande"),
    ("c20", "Mets-moi de la musique douce.", "commande"),
    ("c21", "Fin de la dictée.", "commande"),
    ("c22", "Corrige le texte.", "commande"),
    ("c23", "Verrouille l’ordinateur.", "commande"),
    ("c24", "Combien de tokens aujourd’hui ?", "commande"),
    // Mot d'éveil seul et formules courtes
    ("w01", "Jarvis.", "eveil"),
    ("w02", "Jarvis, tu m’entends ?", "eveil"),
    ("w03", "Merci Jarvis.", "eveil"),
    ("w04", "Stop.", "eveil"),
    ("w05", "Oui, vas-y.", "eveil"),
    // Noms propres et vocabulaire
    ("n01", "Envoie un message à Caroline pour lui dire que j’arrive dans 20 minutes.", "noms"),
    ("n02", "Charlotte a 25 ans et Abigail en a 16.", "noms"),
    ("n03", "Ouvre le projet Worganic dans Visual Studio Code.", "noms"),
    ("n04", "Passe en moteur Claude Code avec le modèle Sonnet.", "noms"),
    ("n05", "Ouvre WhatsApp et YouTube.", "noms"),
    // Phrases libres (questions à l'IA)
    ("l01", "Explique-moi simplement comment fonctionne une pompe à chaleur.", "libre"),
    ("l02", "Donne-moi une idée de recette rapide pour ce soir avec des courgettes.", "libre"),
    ("l03", "Est-ce que tu peux résumer la réunion de ce matin en trois points ?", "libre"),
    ("l04", "Rappelle-moi d’acheter du pain en rentrant.", "libre"),
    ("l05", "Écris un court message pour souhaiter bon anniversaire à ma fille.", "libre"),
    ("l06", "Quelle est la différence entre un crédit à taux fixe et un crédit à taux variable ?", "libre"),
    // Nombres, dates, dictée
    ("d01", "Le rendez-vous est fixé au vendredi 27 septembre à 9h15.", "dictee"),
    ("d02", "Bonjour à tous, virgule, je vous écris pour confirmer notre réunion, point.", "dictee"),
    ("d03", "Le montant total s’élève à 1249,90 €.", "dictee"),
    ("d04", "Nouveau paragraphe. Merci de me renvoyer le document signé avant lundi.", "dictee"),
];

pub fn corpus_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("voice-corpus")
}

pub fn phrases_json() -> Vec<J> {
    PHRASES.iter().map(|(id, text, cat)| json!({ "id": id, "text": text, "category": cat })).collect()
}

/// État du corpus pour l'administration : phrases, prises par configuration, série négative.
pub fn corpus_state(data_dir: &Path) -> J {
    let dir = corpus_dir(data_dir);
    let manifest = js::read_json(&dir.join("manifest.json")).unwrap_or(json!({ "items": [] }));
    let status = js::read_json(&dir.join("status.json"));
    let items = js::arr(&manifest["items"]);
    let mut by_phrase: BTreeMap<String, serde_json::Map<String, Value>> = BTreeMap::new();
    for it in items.iter().filter(|i| js::gs(i, "kind") == "phrase") {
        let cfg = if js::truthy(&it["config"]) { js::gs(it, "config") } else { "autre".into() };
        by_phrase.entry(js::gs(it, "id")).or_default().insert(cfg, json!({ "duration": it["duration"], "at": it["at"] }));
    }
    let negatives: Vec<&J> = items.iter().filter(|i| js::gs(i, "kind") == "negatif").collect();
    let seconds: f64 = negatives.iter().map(|n| js::gn_or(n, "duration", 0.0)).sum();
    let report = js::read_json(&dir.join("report.json")).unwrap_or(Value::Null);
    // Rapport de calibrage : seulement s'il a été calculé avec l'extracteur d'empreinte actuel.
    let mut speaker_report = js::read_json(&dir.join("speaker-report.json")).filter(|r| js::gs(r, "embedder") == SPEAKER_EMBEDDER).unwrap_or(Value::Null);
    if let Some(tr) = speaker_report.get_mut("tranches").and_then(|t| t.as_object_mut()) {
        for t in tr.values_mut() {
            if let Some(o) = t.as_object_mut() {
                o.remove("scores_phrases");
                o.remove("scores_negatifs");
            }
        }
    }
    let now_s = js::now_ms() as f64 / 1000.0;
    let status = status.filter(|s| now_s - js::gn_or(s, "at", 0.0) < 120.0).unwrap_or(Value::Null);
    let phrases: Vec<J> = PHRASES
        .iter()
        .map(|(id, text, cat)| json!({ "id": id, "text": text, "category": cat, "takes": by_phrase.get(*id).cloned().map(Value::Object).unwrap_or(json!({})) }))
        .collect();
    json!({
        "phrases": phrases,
        "negatives": { "count": negatives.len(), "seconds": seconds.round() as i64 },
        "status": status, "report": report, "speakerReport": speaker_report,
        "dir": dir.to_string_lossy()
    })
}

/// Suppression complète des enregistrements (demandée par l'utilisateur).
pub fn clear_corpus(data_dir: &Path) -> usize {
    fn walk(d: &Path, removed: &mut usize) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                walk(&p, removed);
            } else if crate::re!(r"(?i)\.(wav|json)$").is_match(&e.file_name().to_string_lossy()) && std::fs::remove_file(&p).is_ok() {
                *removed += 1;
            }
        }
    }
    let dir = corpus_dir(data_dir);
    let mut removed = 0;
    if dir.exists() {
        walk(&dir, &mut removed);
    }
    removed
}
