// Pré-filtrage des demandes (orchestrator/prefilter.js) : tout ce qui n'a pas besoin d'un modèle ne
// doit jamais en solliciter un. Sur un texte déjà replié (minuscules, sans accents) :
//  - normalize_request : retire l'enrobage (« est-ce que tu peux me… s'il te plaît ») ;
//  - detect_noise : phrases inventées par Whisper (boucles, génériques de vidéos) ;
//  - small_talk : politesses et acquittements, auxquels Jarvis répond seul et brièvement.

use std::sync::LazyLock;

use serde_json::{json, Value};

use super::config::Forms;
use crate::js::{self, J};

pub fn trim_end(text: &str) -> String {
    let t = crate::re!(r"[.…]+").replace_all(text, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

fn imperative(word: &str) -> Option<&'static str> {
    Some(match word {
        "mettre" => "mets",
        "remettre" => "remets",
        "lancer" => "lance",
        "relancer" => "relance",
        "ouvrir" => "ouvre",
        "fermer" => "ferme",
        "demarrer" => "demarre",
        "donner" => "donne",
        "dire" => "dis",
        "redire" => "redis",
        "jouer" => "joue",
        "reprendre" => "reprends",
        "baisser" => "baisse",
        "monter" => "monte",
        "augmenter" => "augmente",
        "diminuer" => "diminue",
        "couper" => "coupe",
        "arreter" => "arrete",
        "afficher" => "affiche",
        "montrer" => "montre",
        "prendre" => "prends",
        "faire" => "fais",
        "lire" => "lis",
        "activer" => "active",
        "desactiver" => "desactive",
        "passer" => "passe",
        "regler" => "regle",
        "chercher" => "cherche",
        "trouver" => "trouve",
        "enregistrer" => "enregistre",
        "supprimer" => "supprime",
        "annuler" => "annule",
        "repeter" => "repete",
        "verifier" => "verifie",
        "basculer" => "bascule",
        "changer" => "change",
        "apprendre" => "apprends",
        "ajouter" => "ajoute",
        "retirer" => "retire",
        "zapper" => "zappe",
        "ecouter" => "ecoute",
        "lister" | "enumerer" => "liste",
        "resumer" => "resume",
        "mais" | "met" => "mets",
        _ => return None,
    })
}

static LEADING: LazyLock<regex::Regex> = LazyLock::new(|| {
    let parts = [
        "jarvis", "dis jarvis", "hey jarvis", "ok jarvis", "eh jarvis", "alors", "bon", "allez", "euh", "ok", "okay", "bah", "ben", "et",
        "si tu (?:peux|veux|pouvais|voulais|voudrais|pourrais)(?: bien)?", "si vous (?:pouvez|voulez|pourriez)(?: bien)?",
        "n hesite pas a", "tu pourrais", "tu voudrais bien", "vous pourriez", "il serait possible de",
        "est ce que tu (?:peux|pourrais|veux bien|voudrais bien|sais)", "est ce que vous (?:pouvez|pourriez|voulez bien)",
        "est ce qu il (?:est possible|serait possible) de", "tu (?:peux|pourrais|veux bien|voudrais bien)", "vous (?:pouvez|pourriez)",
        "(?:peux|pourrais|veux) tu", "(?:pouvez|pourriez) vous", "je (?:veux|voudrais|souhaite|souhaiterais) que tu", "j aimerais que tu",
        "il faudrait que tu", "il faut que tu", "je (?:veux|voudrais|souhaite|souhaiterais)", "j aimerais", "merci de",
    ];
    regex::Regex::new(&format!(r"^(?:{})\s+", parts.join("|"))).unwrap()
});

pub fn normalize_request(folded: &str) -> String {
    let mut text = trim_end(folded);
    loop {
        let previous = text.clone();
        text = crate::fre!(r"^(?:bonjour|salut|bonsoir|coucou|hello|hey)\s+(?=\S+\s+\S+)").replace(&text, "").into_owned();
        text = LEADING.replace(&text, "").into_owned();
        text = crate::re!(r"\s+(?:s il te plait|s il vous plait|stp|svp|merci(?: beaucoup| bien)?|jarvis|please)$").replace(&text, "").into_owned();
        if text == previous || text.is_empty() {
            break;
        }
    }
    if text.is_empty() {
        return trim_end(folded);
    }
    let t = crate::re!(r"\b(?:euh|hum|ben|bah)\b").replace_all(&text, " ");
    let mut text = crate::re!(r"\s+").replace_all(&t, " ").trim().to_string();
    text = crate::re!(r"\bdla\b").replace_all(&text, "de la").into_owned();
    text = crate::re!(r"\bplayliste\b").replace_all(&text, "playlist").into_owned();
    text = crate::re!(r"\b(?:spootify|spotifi|spoty)\b").replace_all(&text, "spotify").into_owned();
    text = crate::re!(r"\b(?:you\s+tube|youtub)\b").replace_all(&text, "youtube").into_owned();
    text = crate::re!(r"\bcroome\b").replace_all(&text, "chrome").into_owned();
    text = crate::re!(r"\bwhapsapp\b").replace_all(&text, "whatsapp").into_owned();
    text = crate::re!(r"\bdiscorde\b").replace_all(&text, "discord").into_owned();
    text = crate::re!(r"^(?:me|m|nous)\s+").replace(&text, "").into_owned();
    let mut words: Vec<String> = text.split(' ').map(String::from).collect();
    if let Some(imp) = words.first().and_then(|w| imperative(w)) {
        words[0] = imp.to_string();
    }
    let text = words.join(" ");
    crate::re!(r"^(\S+)\s+(?:moi|nous)\s+").replace(&text, "$1 ").trim().to_string()
}

fn repeats_in_loop(words: &[&str]) -> bool {
    for n in 1..=6 {
        let mut start = 0;
        while start + n * 3 <= words.len() {
            let unit = words[start..start + n].join(" ");
            let mut count = 1;
            let mut pos = start + n;
            while pos + n <= words.len() && words[pos..pos + n].join(" ") == unit {
                count += 1;
                pos += n;
            }
            if count >= 3 && (n > 1 || count >= 5) {
                return true;
            }
            start += 1;
        }
    }
    false
}

/// Le motif si le texte est un bruit à ignorer.
pub fn detect_noise(folded: &str) -> Option<&'static str> {
    let text = trim_end(folded);
    if text.is_empty() {
        return Some("vide");
    }
    if !crate::re!(r"(?i)[a-z0-9]").is_match(&text) {
        return Some("ponctuation isolée sans parole");
    }
    if text.chars().count() < 2 {
        return Some("bruit trop court");
    }
    let phantoms = [
        crate::re!(r"\bsous titres? (?:realises?|faits?|par)\b"),
        crate::re!(r"\bsous titrage\b"),
        crate::re!(r"\bamara org\b"),
        crate::re!(r"\bmerci d avoir regarde\b"),
        crate::re!(r"\b(?:n oubliez pas de )?(?:vous )?abonnez vous\b"),
        crate::re!(r"\blike(?:z)? et abonne"),
        crate::re!(r"\ba la prochaine pour une nouvelle video\b"),
        crate::re!(r"\bmerci de (?:votre|m avoir) (?:attention|ecoute)\b.*\bvideo\b"),
    ];
    if phantoms.iter().any(|re| re.is_match(&text)) {
        return Some("générique de vidéo inventé par la transcription");
    }
    let words: Vec<&str> = text.split(' ').collect();
    if words.len() >= 6 && repeats_in_loop(&words) {
        return Some("boucle de répétition de la transcription");
    }
    None
}

fn pick(list: &[&str]) -> String {
    list[js::rand_below(list.len())].to_string()
}

/// Politesses, acquittements et rétractions : { kind, say } (say null : silence).
pub fn small_talk(folded: &str, a: &Forms) -> Option<J> {
    let t = trim_end(folded);
    let text = crate::re!(r"^jarvis\s+|\s+jarvis$").replace_all(&t, "").trim().to_string();
    if text.is_empty() || text == "jarvis" {
        return Some(json!({ "kind": "presence", "say": "Oui ?" }));
    }
    if crate::re!(r"(?i)(?:^|[\s,;.!?])(?:non|attends|arrete|stop|oups)(?:[\s,;.!?]+(?:en fait|finalement))?[\s,;.!?]+(?:rien(?: du tout)?|annule(?: tout)?|laisse tomber|pas la peine|oublie)(?:[\s,;.!?]+(?:annule|stop))?[\s.!?]*$").is_match(&text)
        || crate::re!(r"(?i)(?:^|[\s,;.!?])(?:annule(?: tout)?|laisse tomber|non rien)[\s.!?]*$").is_match(&text)
    {
        return Some(json!({ "kind": "cancel", "say": "Action annulée." }));
    }
    if crate::re!(r"^(?:(?:ok|super|parfait|top|genial|tres bien|d accord)\s+)?(?:merci|merci (?:beaucoup|bien|infiniment|a toi|a vous|jarvis)|je (?:te|vous) remercie|c est gentil(?: merci)?)$").is_match(&text) {
        return Some(json!({ "kind": "thanks", "say": pick(&["Avec plaisir.", if a.vous { "Je vous en prie." } else { "De rien." }, if a.vous { "À votre service." } else { "À ton service." }]) }));
    }
    if crate::re!(r"^(?:ok|okay|d accord|parfait|super|genial|top|tres bien|nickel|cool|ca marche|entendu|c est bon|c est parti|bien recu|compris|ok parfait|ok super|ok d accord|impeccable|excellent|bien)$").is_match(&text) {
        return Some(json!({ "kind": "ack", "say": pick(&["Très bien.", "Entendu.", "Parfait."]) }));
    }
    if crate::re!(r"^(?:(?:est ce que )?(?:tu es|es tu|t es|vous etes|etes vous) (?:la|toujours la|reveille)|(?:est ce que )?tu (?:m )?(?:entends|ecoutes)|vous m (?:entendez|ecoutez)|m entends tu|tu es ou)$").is_match(&text) {
        return Some(json!({ "kind": "presence", "say": format!("Oui, je {}.", a.cod("écoute")) }));
    }
    if crate::re!(r"^(?:(?:et )?(?:comment )?(?:ca va|vas tu|tu vas bien|allez vous|vous allez bien)|comment ca va|comment vas tu|comment allez vous)$").is_match(&text) {
        return Some(json!({ "kind": "howareyou", "say": "Tous mes systèmes sont opérationnels, merci." }));
    }
    if let Some(bye) = crate::re!(r"^(?:au revoir|ciao|bye|salut|a plus|a bientot|a demain|bonne (nuit|soiree|journee))$").captures(&text) {
        if text != "salut" {
            let say = match bye.get(1).map(|m| m.as_str()) {
                Some("nuit") => "Bonne nuit.",
                Some("soiree") => "Bonne soirée.",
                Some("journee") => "Bonne journée.",
                _ => "À bientôt.",
            };
            return Some(json!({ "kind": "bye", "say": say }));
        }
    }
    if crate::re!(r"^(?:(?:ok|non|allez|stop|chut)\s+)*(?:stop|tais toi|taisez vous|chut|ca suffit|arrete de parler|laisse tomber|non rien|rien|oublie|c est pas grave|pas grave|non merci|non c est bon)$").is_match(&text) {
        return Some(json!({ "kind": "hush", "say": Value::Null }));
    }
    None
}

#[cfg(test)]
mod tests_politesse {
    use super::*;
    use crate::orch::answer_cache::fold;

    fn norm(s: &str) -> String {
        normalize_request(&fold(s))
    }

    #[test]
    fn si_tu_peux_est_retire_de_la_demande() {
        assert_eq!(norm("Jarvis, si tu peux me donner les derniers mails que j'ai reçus."), "donne les derniers mails que j ai recus");
        assert_eq!(norm("si tu veux bien me lire mes mails"), "lis mes mails");
        assert_eq!(norm("Si vous pouvez m'afficher mes mails"), "affiche mes mails");
        assert_eq!(norm("n'hésite pas à lancer la musique"), "lance la musique");
    }

    #[test]
    fn les_formules_deja_couvertes_ne_changent_pas() {
        assert_eq!(norm("est-ce que tu peux me donner mes mails"), "donne mes mails");
        assert_eq!(norm("peux-tu lancer la musique"), "lance la musique");
        assert_eq!(norm("lis mes mails"), "lis mes mails");
    }
}
