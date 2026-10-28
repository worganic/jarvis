// Texte reconnu : amorce de Whisper, corrections phonétiques, début de commande et filtres
// anti-hallucination (fonctions de voice/service.py reprises par le moteur de la v3).

use fancy_regex::{Captures, Regex};

use crate::util::J;

pub const SR: usize = 16000;

/// Whisper invente ces phrases quand il n'entend que du bruit.
const JUNK: &[&str] = &["sous-titrage", "sous-titres", "merci d'avoir regardé", "abonnez-vous", "amara.org", "radio-canada"];

/// Vocabulaire orientant Whisper, complété à chaud par l'orchestrateur (message « vocab »).
pub const BASE_VOCABULARY: &[&str] = &[
    "Jarvis", "Claude Code", "AGY", "Gemini", "Electron", "VS Code", "Git", "GitHub",
    "PowerShell", "WebSocket", "commit", "push", "pull", "capture d'écran", "journal scellé",
    "minuteur", "plein écran", "prompt", "je valide le prompt",
    "Spotify", "Chrome", "Discord", "YouTube", "Notepad", "Steam", "WhatsApp", "VLC",
    "Word", "Excel", "Calculatrice", "Terminal", "gmeet", "Google Meet", "Meet",
    "playlist", "chanson", "morceau", "artiste", "album", "volume", "luminosité",
];
const PROMPT_MAX_CHARS: usize = 600;
/// L'amorce montre à Whisper des commandes à l'impératif, bien ponctuées.
const COMMAND_EXAMPLES: &str = "Jarvis, mets de la musique. Lis mes derniers mails. Ouvre Discord. \
Quelle heure est-il ? Baisse le son. Cherche le fichier devis. Je valide le prompt. ";
const HOTWORDS_MAX_CHARS: usize = 200;

/// Amorce : exemples de commandes, puis le vocabulaire (termes de l'utilisateur d'abord).
pub fn build_initial_prompt(terms: &[String]) -> String {
    let mut vocab: Vec<String> = vec![];
    let mut seen = std::collections::HashSet::from(["jarvis".to_string()]);
    for term in terms.iter().map(|s| s.as_str()).chain(BASE_VOCABULARY.iter().copied()) {
        let t = term.trim();
        if !t.is_empty() && seen.insert(t.to_lowercase()) {
            vocab.push(t.to_string());
        }
    }
    let mut prompt = format!("{COMMAND_EXAMPLES}Vocabulaire : ");
    for (i, term) in vocab.iter().enumerate() {
        let piece = if i == 0 { term.clone() } else { format!(", {term}") };
        if prompt.chars().count() + piece.chars().count() + 1 > PROMPT_MAX_CHARS {
            break;
        }
        prompt.push_str(&piece);
    }
    prompt + "."
}

/// Mots prioritaires (« hotwords ») : les termes propres à l'utilisateur, pas le vocabulaire de base.
pub fn build_hotwords(terms: &[String]) -> Option<String> {
    let mut out: Vec<String> = vec![];
    let mut seen = std::collections::HashSet::new();
    for term in terms {
        let t = term.trim();
        if t.is_empty() || seen.contains(&t.to_lowercase()) {
            continue;
        }
        let mut probe = out.clone();
        probe.push(t.to_string());
        if probe.join(" ").chars().count() <= HOTWORDS_MAX_CHARS {
            seen.insert(t.to_lowercase());
            out.push(t.to_string());
        }
    }
    (!out.is_empty()).then(|| out.join(" "))
}

/// Mots isolés souvent hallucinés sur du souffle ou du silence.
const ISOLATED_HALLUCINATIONS: &[&str] = &[
    "merci.", "merci beaucoup.", "merci", "merci beaucoup", "de rien.", "de rien", "au revoir.", "au revoir",
    "bonjour.", "bonjour", "sous-titrage.", "sous-titres.", "à bientôt.", "a bientot.", "bonne journée.", "bonne nuit.",
];

/// Remplacement qui garde la majuscule du mot remplacé (« Montre » → « Monte »).
fn same_case(text: &str, start: usize, new: &str) -> String {
    let upper = text[start..].chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
    if upper {
        let mut c = new.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    } else {
        new.to_string()
    }
}

type Repl = Box<dyn Fn(&Captures) -> String + Send + Sync>;

fn rules() -> &'static Vec<(Regex, Repl)> {
    static RULES: std::sync::LazyLock<Vec<(Regex, Repl)>> = std::sync::LazyLock::new(|| {
        let lit = |s: &'static str| -> Repl { Box::new(move |_| s.to_string()) };
        let grp1 = |suffix: &'static str| -> Repl { Box::new(move |c: &Captures| format!("{}{suffix}", c.get(1).map(|m| m.as_str()).unwrap_or(""))) };
        let r = |p: &str| Regex::new(p).unwrap();
        vec![
            (r(r"(?i)\b(?:charvis|j'arvis|jarvisse)\b"), lit("Jarvis")),
            (r(r"(?i)\b(?:clode|claude\s+kod|claudecode)\b"), lit("Claude Code")),
            (r(r"(?i)\ba\s+g\s+y\b"), lit("AGY")),
            (r(r"(?i)\bv\s+s\s+code\b"), lit("VS Code")),
            (r(r"(?i)\bpower\s+shell\b"), lit("PowerShell")),
            (r(r"(?i)\b(?:spootify|spotifi|spoti\s*faï|spoty)\b"), lit("Spotify")),
            (r(r"(?i)\bdla\b"), lit("de la")),
            (r(r"(?i)\bplayliste\b"), lit("playlist")),
            (r(r"(?i)\b(?:you\s+tube|youtub)\b"), lit("YouTube")),
            (r(r"(?i)\b(?:croome|chraume)\b"), lit("Chrome")),
            (r(r"(?i)\b(?:whapsapp|ouatsap|watsap)\b"), lit("WhatsApp")),
            (r(r"(?i)\bdiscorde\b"), lit("Discord")),
            (r(r"(?i)\b(?:g\s+mit|g\s*-\s*mit|gmit|gémit|gémis|g\s+meet|g-meet)\b"), lit("gmeet")),
            // « mails » entendu « mets », « pause » entendu « pose ».
            (r(r"(?i)\b((?:cherche|lis|trouve|affiche|montre|consulte|ouvre|relève|releve)\s+(?:les|mes|des|le|mon)\s+(?:derniers?\s+|nouveaux?\s+)?)(?:mets|mèls?|mels?)\b"), grp1("mails")),
            (r(r"(?i)\b((?:musique|chanson|morceau|lecture|spotify|vidéo|video)\s+en\s+)pose\b"), grp1("pause")),
            // Homophones de commandes de lecture et de volume, chacun dans un contexte sans ambiguïté.
            (
                r(r"(?i)\bmontre(?=\s+le\s+(?:volume|son)\s+(?:à|a)\s+(?:\d|zéro|cinq|dix|vingt|trente|quarante|cinquante|soixante|quatre|cent))"),
                Box::new(|c: &Captures| {
                    let m = c.get(0).unwrap();
                    same_case(m.as_str(), 0, "monte")
                }),
            ),
            (
                r(r"(?i)^(\s*jarvis[\s,.!?]*)?laisse(?=\s+le\s+(?:son|volume)\s*(?:[,.!?]|s['’]il|un\s+peu|$))"),
                Box::new(|c: &Captures| {
                    let g1 = c.get(1).map(|m| m.as_str()).unwrap_or("");
                    let whole = c.get(0).unwrap().as_str();
                    format!("{g1}{}", same_case(whole, g1.len(), "baisse"))
                }),
            ),
            (r(r"(?i)\b((?:mets|met|mais)\s+(?:la\s+)?(?:musique|chanson|lecture|vidéo|video)\s+)compos[ée]e?(?!\s+(?:par|de|d['’]|pour|avec|en|sur|à)\b)\b"), grp1("en pause")),
            (
                r(r"(?i)^(\s*jarvis[\s,.!?]*)?(?:à|a)(?=\s+la\s+(?:chanson|musique|piste|vidéo|video)\s+(?:suivante|précédente|precedente)\b)"),
                Box::new(|c: &Captures| {
                    let g1 = c.get(1).map(|m| m.as_str()).unwrap_or("");
                    let whole = c.get(0).unwrap().as_str();
                    format!("{g1}{}", same_case(whole, g1.len(), "passe à"))
                }),
            ),
        ]
    });
    &RULES
}

/// Corrections apprises « entendu → correct » (mots entiers, casse ignorée).
pub fn compile_corrections(pairs: &J) -> Vec<(Regex, String)> {
    let mut out = vec![];
    for pair in pairs.as_array().cloned().unwrap_or_default() {
        let heard = pair["entendu"].as_str().unwrap_or("").trim().to_string();
        let right = pair["correct"].as_str().unwrap_or("").trim().to_string();
        if heard.is_empty() || right.is_empty() {
            continue;
        }
        let words: Vec<String> = heard.split_whitespace().map(fancy_regex::escape).map(|c| c.into_owned()).collect();
        let pattern = format!(r"(?i)(?<!\w){}(?!\w)", words.join(r"\s+"));
        if let Ok(re) = Regex::new(&pattern) {
            out.push((re, right));
        }
    }
    out
}

pub fn clean_and_autocorrect(text: &str, extra: &[(Regex, String)]) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let mut res = text.trim().to_string();
    for (re, repl) in rules() {
        res = re.replace_all(&res, |c: &Captures| repl(c)).to_string();
    }
    for (re, right) in extra {
        // Remplacement littéral : un « $ » ou « \ » du mot appris n'est pas interprété.
        res = re.replace_all(&res, fancy_regex::NoExpand(right)).to_string();
    }
    fix_command_start(&res)
}

// --- Début de commande ---------------------------------------------------------------------------

/// Mot d'éveil mal entendu en tête ; « J'arrive » seul ou suivi d'une commande ; homophones de
/// l'impératif (« Jarvis met de la musique »).
pub fn fix_command_start(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let wake_start = crate::fre!(r"(?i)^\s*(?:j['’]arvis|jarvi|jarvisse|jarvise|charvis|jervis|jarvice)(?=[\s,.!?]|$)[\s,.!?]*");
    let jarrive = crate::fre!(
        r"(?i)^\s*j['’]arrive(?:\s*[,.!?]+\s*|\s*$|\s+(?=(?:mets|lance|ouvre|lis|quelle|quel|est-ce|dis|cherche|joue|fais|baisse|monte|arrête|arrete|coupe|peux|tu|affiche|montre|trouve|ferme|passe)\b))"
    );
    let wake_only = crate::fre!(r"(?i)^\s*jarvis\b[\s,.!?]*");
    let fixes: [(&Regex, &str); 5] = [
        (crate::fre!(r"(?i)^(?:met|mais|mè)(?=\s+(?:de|du|des|la|le|les|un|une|moi|nous|en|pause|plus|ça|ca|sur)\b)"), "mets"),
        (crate::fre!(r"(?i)^lit(?=\s+(?:mon|ma|mes|le|la|les|un|une|moi|ce|cette|ces)\b)"), "lis"),
        (crate::fre!(r"(?i)^fait(?=\s+(?:un|une|le|la|les|moi|nous|des|du|de|pause)\b)"), "fais"),
        (crate::fre!(r"(?i)^mémoire(?=\s+(?:de|du|des|la|le)\s)"), "mets-moi"),
        (
            crate::fre!(r"(?i)^créer(?=\s+(?:un|une|le|la|les|des|mon|ma|mes)\s+(?:nouveau\s+|nouvelle\s+)?(?:google|doc|document|fichier|dossier|rendez-vous|rdv|note|playlist|minuteur|rappel|raccourci|tableau|dossier|événement|evenement)\b)"),
            "crée",
        ),
    ];
    let res = wake_start.replacen(text, 1, "Jarvis, ").to_string();
    let res = jarrive.replacen(&res, 1, "Jarvis, ").to_string();
    let (mut head, mut body) = match wake_only.find(&res).ok().flatten() {
        Some(m) => (res[..m.end()].to_string(), res[m.end()..].to_string()),
        None => (String::new(), res.clone()),
    };
    for (re, repl) in fixes {
        let new = re.replacen(&body, 1, repl).to_string();
        if new != body {
            body = if head.is_empty() {
                let mut c = new.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            } else {
                new
            };
            break;
        }
    }
    if !head.is_empty() && !body.is_empty() && !head.trim_end().ends_with([',', '.', '!', '?']) {
        head = format!("{}, ", head.trim_end());
    }
    format!("{head}{body}").trim().to_string()
}

// --- Filtres anti-hallucination ------------------------------------------------------------------

pub fn norm_words(text: &str) -> Vec<String> {
    crate::re!(r"[a-zà-öø-ÿ0-9']+").find_iter(&text.to_lowercase()).map(|m| m.as_str().to_string()).collect()
}

/// Boucle d'hallucination : même mot ≥ 6 fois d'affilée, ou même phrase ≥ 3 fois.
pub fn repeated_loop(text: &str) -> bool {
    let words = norm_words(text);
    let mut run = 1;
    for i in 1..words.len() {
        run = if words[i] == words[i - 1] { run + 1 } else { 1 };
        if run >= 6 {
            return true;
        }
    }
    let phrases: Vec<String> = crate::re!(r"[.,;!?]+").split(text).map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect();
    let set: std::collections::HashSet<&String> = phrases.iter().collect();
    set.iter().any(|p| p.chars().count() >= 3 && phrases.iter().filter(|q| q == p).count() >= 3)
}

/// Le texte reprend-il (≥ 80 % des mots) une phrase dite récemment par Jarvis ?
pub fn similar_to(text: &str, phrases: &[String]) -> bool {
    let words: std::collections::HashSet<String> = norm_words(text).into_iter().collect();
    if words.is_empty() {
        return false;
    }
    phrases.iter().any(|p| {
        let r: std::collections::HashSet<String> = norm_words(p).into_iter().collect();
        !r.is_empty() && words.intersection(&r).count() as f64 / words.len() as f64 >= 0.8
    })
}

/// Le texte contient-il assez de mots anglais courants pour vérifier la langue ?
pub fn english_hint(text: &str) -> bool {
    crate::re!(r"(?i)\b(?:the|we|you|i'm|it's|that|this|what|got|have|and|of|is|are|with|for|my|your|thank|thanks)\b").find_iter(text).count() >= 2
}

/// Filtres anti-hallucination : (texte retenu ou None, motif du rejet).
pub fn assess(text: &str, info: &crate::whisper::Decoded, audio_len: usize, final_: bool, recent_said: &[String], speaker_score: Option<f64>, speaker_threshold: Option<f64>) -> (Option<String>, String) {
    if text.chars().count() < 2 {
        return (None, "vide".into());
    }
    if !crate::re!(r"[a-zA-Z0-9àâäéèêëîïôöùûüçÀÂÄÉÈÊËÎÏÔÖÙÛÜÇ]").is_match(text) {
        return (None, "ponctuation seule".into());
    }
    let low = text.to_lowercase();
    if JUNK.iter().any(|j| low.contains(j)) {
        return (None, "phrase parasite connue".into());
    }
    if !final_ {
        return (Some(text.to_string()), String::new());
    }
    let clean_lower = low.trim().to_string();
    let duration = audio_len as f64 / SR as f64;
    if repeated_loop(text) {
        return (None, "boucle de répétition".into());
    }
    if info.compression > 2.4 {
        return (None, format!("taux de compression {:.1}", info.compression));
    }
    if !recent_said.is_empty() && similar_to(text, recent_said) {
        return (None, "écho d'une phrase de Jarvis".into());
    }
    if ISOLATED_HALLUCINATIONS.contains(&clean_lower.as_str()) && (duration < 1.3 || info.no_speech > 0.40) {
        return (None, "hallucination isolée sur souffle".into());
    }
    if crate::re!(r"(?i)^(?:merci(?: beaucoup| bien| à vous| à tous| à toi)?[.!]?\s*)+$").is_match(text.trim()) {
        let weak = matches!((speaker_score, speaker_threshold), (Some(s), Some(t)) if s < t + 0.05);
        if duration < 1.2 || info.no_speech > 0.30 || info.logprob < -0.6 || weak {
            return (None, "« merci » isolé peu fiable".into());
        }
    }
    if info.no_speech > 0.65 {
        return (None, format!("probabilité de silence {:.2}", info.no_speech));
    }
    if info.logprob < -1.0 {
        return (None, format!("confiance faible {:.2}", info.logprob));
    }
    (Some(text.to_string()), String::new())
}

/// Phrases découpées pour la synthèse (fin de phrase suivie d'un espace).
pub fn split_sentences(text: &str) -> Vec<String> {
    let t = text.trim();
    let mut out = vec![];
    let mut start = 0;
    for m in crate::fre!(r"(?<=[.!?…:])\s+").find_iter(t).flatten() {
        let s = t[start..m.start()].trim();
        if !s.is_empty() {
            out.push(s.to_string());
        }
        start = m.end();
    }
    let s = t[start..].trim();
    if !s.is_empty() {
        out.push(s.to_string());
    }
    out
}

/// Fin de phrase qui appelle une suite (mot de liaison, article, hésitation, verbe d'action sans
/// complément…) : le moteur attend la suite au lieu de clore la phrase.
pub fn looks_unfinished(text: &str) -> bool {
    let raw = text.trim_end();
    let t = if raw.ends_with("...") { raw.to_string() } else { crate::re!(r#"[\s.!?»"”]+$"#).replace(text.trim(), "").to_string() };
    !t.is_empty()
        && js_test(
            crate::fre!(
                r"(?i)(?:(?:^|\s)(?:et|ou|mais|donc|puis|ensuite|alors|car|parce|que|qu['’]|qui|quoi|dont|où|si|quand|comme|pour|par|sur|sous|dans|avec|sans|chez|vers|entre|après|avant|de|du|des|d['’]|le|la|les|l['’]|un|une|au|aux|à|a|en|mon|ma|mes|ton|ta|tes|son|sa|ses|notre|votre|leur|leurs|ce|cet|cette|ces|je|j['’]|tu|il|elle|on|nous|vous|ils|elles|me|m['’]|te|t['’]|se|lui|ne|très|trop|plus|genre|euh+|heu+|hum+|hm+|bah|ben|bon|enfin|voilà|disons|c['’]est|est-ce|faut|veux|voudrais|voudrait|peux|pourrais|dois|vais|ouvre|lance|mets|mettre|ferme|cherche|trouve|montre|affiche|dis|dis-moi|écris|crée|envoie|appelle|rappelle-moi|projet|fichier|dossier|application|appli|fenêtre|card|carte|numéro|playlist|chanson|artiste)|[,:;…-]|\.\.\.)$"
            ),
            &t,
        )
}

fn js_test(re: &Regex, t: &str) -> bool {
    re.is_match(t).unwrap_or(false)
}

pub fn wake_word(text: &str) -> bool {
    crate::re!(r"(?i)^\s*(?:(?:dis|hey|ok|bonjour|salut|bonsoir)\s+)?(?:jarvis|jervis|jarvice|charvis|jarvi|jarvisse|jarvise|garvis|djarvis|j['’]arvis)\b").is_match(text)
}

pub fn wake_only(text: &str) -> bool {
    crate::re!(r"(?i)^\s*(?:(?:dis|hey|ok|bonjour|salut|bonsoir)\s+)?(?:jarvis|jervis|jarvice|charvis|jarvi|jarvisse|jarvise|j['’]arvis)\s*[?!.,…]*\s*$").is_match(text)
}
