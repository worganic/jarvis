// Module « Agents » : agents autonomes créés à partir d'une demande en langage naturel (zone de texte ou voix).
// Architecture hybride (capteurs locaux en temps réel à 0 jeton + exécution IA ciblée) :
//   - Capteurs locaux (Rust) : surveillance continue du matériel (CPU, RAM, GPU, température), des horaires
//     (quotidien à heure fixe ou intervalle régulier), des nouveaux mails (Gmail) et des demandes à la voix.
//   - Exécution IA : modèles minimum Gemini 3.8 Flash pour AGY (gemini-3.8-flash-low / medium / high)
//     ou Claude Haiku (haiku / sonnet / opus) pour Claude Code, avec bascule automatique si quota épuisé.
//   - Restitution : Voix + Card HUD (« voix »), Card HUD seule (« visuel »), ou Historique seul (« silencieux »),
//     avec envoi optionnel sur Telegram.
// Stockage : data/agents.json.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{Datelike, Local, Timelike};
use serde_json::{json, Value};

use super::intent::Intent;
use super::local_skills::parse_time;
use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

const MAX_AGENTS: usize = 60;
const MAX_HISTORIQUE: usize = 15;
const TICK_MS: u64 = 10_000;

/// Modèles autorisés (minimum 3.8 Flash pour AGY, minimum Haiku pour Claude Code).
pub const AGY_MODELS: &[(&str, &str)] = &[
    ("gemini-3.8-flash-high", "AGY · Gemini 3.8 Flash High"),
    ("gemini-3.8-flash-medium", "AGY · Gemini 3.8 Flash Medium"),
    ("gemini-3.8-flash-low", "AGY · Gemini 3.8 Flash Low"),
    ("gemini-3.1-pro-high", "AGY · Gemini 3.1 Pro High"),
];

pub const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("haiku", "Claude · Haiku 4.5"),
    ("sonnet", "Claude · Sonnet 4.6"),
    ("opus", "Claude · Opus 4.7"),
];

const PARSE_SYSTEM_PROMPT: &str = "Tu es l'architecte d'agents autonomes de Jarvis. \
À partir de la demande en français de l'utilisateur, construis la configuration JSON d'un agent de surveillance et d'action. \
Réponds uniquement par un objet JSON valide, sans texte autour ni bloc markdown : \
{ \
  \"nom\": \"Nom court (35 car. max)\", \
  \"icone\": \"Un symbole ou emoji (ex: ✉, ⚙, 🏉, 🌐, 🔔)\", \
  \"description\": \"Résumé clair de ce que surveille et fait l'agent (140 car. max)\", \
  \"declencheurs\": { \
    \"horaire\": { \"actif\": true/false, \"type\": \"quotidien\" ou \"intervalle\", \"heure\": \"HH:MM\", \"minutes\": 60, \"jours\": [1,2,3,4,5,6,7] }, \
    \"materiel\": { \"actif\": true/false, \"cpuPct\": 85, \"ramPct\": 85, \"gpuPct\": 85, \"gpuTemp\": 83, \"dureeSecondes\": 15, \"cooldownMinutes\": 30 }, \
    \"mails\": { \"actif\": true/false, \"mode\": \"resume_jour\" ou \"nouveau_mail\", \"filtre\": \"\" }, \
    \"demande\": { \"actif\": true/false, \"motsCles\": [\"mot1\", \"mot2\"] } \
  }, \
  \"sources\": [\"mails\", \"materiel\", \"web\", \"agenda\"], \
  \"consigneIa\": \"Consigne précise pour l'IA lors de l'exécution de l'agent\" \
}. \
Règles : \
1. Si la demande parle de surveiller les mails et les résumer chaque jour : active declencheurs.horaire (quotidien, ex: 18:00 ou l'heure demandée) et declencheurs.mails (mode resume_jour), sources=[\"mails\"]. \
2. Si la demande parle de surveiller le PC (RAM, CPU, GPU) et conseiller des optimisations : active declencheurs.materiel avec les seuils appropriés (par défaut 85%), sources=[\"materiel\"]. \
3. Si la demande parle de sport (ex: rugby Top 14), d'actualité, de programme TV ou de recherche : active declencheurs.horaire si 'chaque jour' est demandé, active declencheurs.demande avec des motsCles pertinents, sources=[\"web\"].";

fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    let t = crate::re!(r#"['’\-?!,;:.…«»"()]"#).replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

/// Garantit que le modèle AGY est au minimum en version 3.8 Flash (ou 3.1 Pro), et Claude au minimum Haiku.
pub fn sanitize_model(engine: &str, model: &str) -> String {
    let m = model.trim().to_lowercase();
    if m.is_empty() || m == "auto" {
        return "auto".into();
    }
    if engine == "agy" || m.starts_with("gemini") || m.starts_with("flash") {
        return match m.as_str() {
            "gemini-3.8-flash-low" | "flash-low" => "gemini-3.8-flash-low".into(),
            "gemini-3.8-flash-medium" | "flash-medium" => "gemini-3.8-flash-medium".into(),
            "gemini-3.8-flash-high" | "flash-high" | "flash" | "gemini-3.8-flash" => "gemini-3.8-flash-high".into(),
            "gemini-3.1-pro-high" | "pro" => "gemini-3.1-pro-high".into(),
            // Tout ancien modèle (3.6, 3.7…) est automatiquement relevé vers 3.8 Flash High.
            _ => "gemini-3.8-flash-high".into(),
        };
    }
    match m.as_str() {
        "haiku" | "claude-haiku" => "haiku".into(),
        "sonnet" | "claude-sonnet" => "sonnet".into(),
        "opus" | "claude-opus" => "opus".into(),
        _ => "haiku".into(),
    }
}

pub fn model_label(engine: &str, model: &str) -> String {
    if engine == "agy" {
        AGY_MODELS
            .iter()
            .find(|(id, _)| *id == model)
            .map(|(_, l)| (*l).to_string())
            .unwrap_or_else(|| "AGY · Gemini 3.8 Flash High".into())
    } else {
        CLAUDE_MODELS
            .iter()
            .find(|(id, _)| *id == model)
            .map(|(_, l)| (*l).to_string())
            .unwrap_or_else(|| "Claude · Haiku 4.5".into())
    }
}

/// Analyse locale déterministe d'une demande en langage naturel : sert de base sûre et de repli immédiat
/// (notamment hors ligne ou en test), et complète les champs manquants d'une proposition IA.
pub fn parse_prompt_local(prompt: &str) -> J {
    let raw = prompt.trim();
    let f = fold(raw);

    let has_mail = crate::re!(r"\b(?:mails?|emails?|courriels?|boite de reception|gmail|messages? recus?)\b").is_match(&f);
    let has_hw = crate::re!(r"\b(?:ram|cpu|gpu|processeur|memoire vive|carte graphique|surchauffe|temperature|performances?| Mon pc|surveille (?:mon |le )?(?:pc|ordinateur|ordi))\b").is_match(&f);
    let has_rugby = crate::re!(r"\b(?:rugby|top ?14|pro ?d2|xv de france)\b").is_match(&f);
    let has_daily = crate::re!(r"\b(?:chaque jour|tous les jours|une fois par jour|quotidien(?:nement)?|par jour|matin|soir)\b").is_match(&f);
    let has_on_demand = crate::re!(r"\b(?:sur demande|a la demande|quand je (?:le )?demande)\b").is_match(&f);

    // Heure éventuelle dans la phrase (« à 18h », « à 8 h 30 »)
    let parsed_hour = parse_time(&f).unwrap_or_else(|| {
        if f.contains("matin") {
            "08:30".into()
        } else if f.contains("midi") {
            "12:00".into()
        } else if has_mail {
            "18:00".into()
        } else {
            "09:00".into()
        }
    });

    // Intervalle éventuel (« toutes les 2 heures », « toutes les 30 minutes »)
    let mut interval_min: Option<i64> = None;
    if let Some(m) = crate::re!(r"\btoutes? les (\d{1,3}) ?(minutes?|min|heures?|h)\b").captures(&f) {
        let n = m[1].parse::<i64>().unwrap_or(60);
        let mult = if m[2].starts_with('h') { 60 } else { 1 };
        interval_min = Some((n * mult).clamp(5, 1440));
    } else if crate::re!(r"\btoutes? les heures\b").is_match(&f) {
        interval_min = Some(60);
    }

    // Seuils matériels éventuels (« plus de 80% », « dépasse 90 % »)
    let pct_threshold = crate::re!(r"(\d{2})\s*%")
        .captures(&f)
        .and_then(|m| m[1].parse::<i64>().ok())
        .map(|n| n.clamp(40, 99))
        .unwrap_or(85);

    let mut sources: Vec<String> = vec![];
    if has_mail {
        sources.push("mails".into());
    }
    if has_hw {
        sources.push("materiel".into());
    }
    if has_rugby || (!has_mail && !has_hw) || crate::re!(r"\b(?:web|internet|actualite|matchs?|programme|chaine|diffusion|meteo|prix|cours)\b").is_match(&f) {
        sources.push("web".into());
    }
    if crate::re!(r"\b(?:agenda|rendez vous|rdv|planning|calendrier)\b").is_match(&f) {
        sources.push("agenda".into());
    }

    let (nom, icone, description, consigne, mots_cles) = if has_mail && !has_hw && !has_rugby {
        (
            "Résumé quotidien des mails".to_string(),
            "✉".to_string(),
            format!("Surveille la boîte Gmail et résume les mails reçus tous les jours à {parsed_hour}."),
            "Analyse les derniers mails reçus fournis dans le contexte. Fais un résumé clair et structuré en français : expéditeurs importants, sujets urgents ou actions attendues, et nombre total de mails. Commence par une synthèse orale de 2 à 3 phrases courtes.".to_string(),
            vec!["resume mes mails".into(), "bilan des mails".into()],
        )
    } else if has_hw && !has_rugby {
        (
            "Sentinelle Performances PC".to_string(),
            "⚙".to_string(),
            format!("Surveille en continu le CPU, la RAM et le GPU (seuil {pct_threshold} %) et conseille des optimisations en cas de surcharge."),
            "À partir des mesures CPU, RAM, GPU et de la liste des processus les plus gourmands fournis dans le contexte, indique clairement quel composant (RAM, CPU ou GPU) est surchargé, cite les logiciels responsables et propose 3 conseils concrets et immédiats pour améliorer les performances du PC.".to_string(),
            vec!["analyse les performances du pc".into(), "pourquoi mon pc rame".into(), "diagnostic ram cpu gpu".into()],
        )
    } else if has_rugby {
        (
            "Veille Rugby TOP 14".to_string(),
            "🏉".to_string(),
            format!("Indique chaque jour à {parsed_hour} et sur demande le programme des matchs du TOP 14 (date, heure et chaîne TV)."),
            "Recherche sur le web le programme à jour des prochains matchs de rugby du TOP 14 (journée en cours ou à venir). Donne la liste claire des matchs avec pour chacun : les équipes (domicile vs extérieur), la date, l'heure exacte du coup d'envoi et la chaîne de diffusion TV (Canal+, Canal+ Sport, Canal+ Live…).".to_string(),
            vec!["matchs du top 14".into(), "programme rugby".into(), "liste des matchs de rugby".into(), "top 14".into()],
        )
    } else {
        let short_name = {
            let cleaned = crate::re!(r"(?i)^(?:surveille|verifie|indique(?:-moi| moi)?|donne(?:-moi| moi)?|j['’]adore|je voudrais|cree un agent qui)\s+")
                .replace(raw, "");
            let s = js::slice_chars(cleaned.trim(), 34);
            if s.is_empty() { "Agent personnalisé".into() } else { js::capitalize(&s) }
        };
        let words: Vec<String> = f
            .split_whitespace()
            .filter(|w| w.len() >= 4 && !["surveille", "indique", "chaque", "jour", "tous", "jours", "une", "fois", "avec", "pour", "dans", "plus", "moins", "comme", "alors", "demande", "liste"].contains(w))
            .take(4)
            .map(String::from)
            .collect();
        (
            short_name,
            "◈".to_string(),
            js::slice_chars(raw, 140),
            format!("Exécute la mission suivante en français de manière précise et concise : {raw}"),
            words,
        )
    };

    let horaire_actif = has_daily || interval_min.is_some() || (has_mail && !f.contains("nouveau mail")) || (!has_hw && !has_on_demand);
    let horaire_type = if interval_min.is_some() { "intervalle" } else { "quotidien" };
    let mails_mode = if f.contains("nouveau") || f.contains("des qu un mail") || f.contains("temps reel") {
        "nouveau_mail"
    } else {
        "resume_jour"
    };

    json!({
        "nom": nom,
        "icone": icone,
        "description": description,
        "declencheurs": {
            "horaire": {
                "actif": horaire_actif,
                "type": horaire_type,
                "heure": parsed_hour,
                "minutes": interval_min.unwrap_or(60),
                "jours": [1, 2, 3, 4, 5, 6, 7]
            },
            "materiel": {
                "actif": has_hw,
                "cpuPct": pct_threshold,
                "ramPct": pct_threshold,
                "gpuPct": pct_threshold,
                "gpuTemp": 83,
                "dureeSecondes": 15,
                "cooldownMinutes": 30
            },
            "mails": {
                "actif": has_mail,
                "mode": mails_mode,
                "filtre": ""
            },
            "demande": {
                "actif": has_on_demand || has_rugby || !mots_cles.is_empty(),
                "motsCles": mots_cles
            }
        },
        "sources": sources,
        "consigneIa": consigne
    })
}

/// Normalise et sécurise un objet Agent complet avant enregistrement.
pub fn normalize_agent(raw: &J, fallback_prompt: &str, existing_id: Option<&str>) -> J {
    let raw_prompt = js::gs(raw, "promptOriginal");
    let base = parse_prompt_local(if fallback_prompt.is_empty() { &raw_prompt } else { fallback_prompt });
    let id = existing_id
        .map(String::from)
        .or_else(|| Some(js::gs(raw, "id")).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| format!("ag_{}", js::now_ms()));

    let nom = Some(js::slice_chars(js::gs(raw, "nom").trim(), 45))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| js::gs(&base, "nom"));
    let icone = Some(js::slice_chars(js::gs(raw, "icone").trim(), 4))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| js::gs(&base, "icone"));
    let description = Some(js::slice_chars(js::gs(raw, "description").trim(), 200))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| js::gs(&base, "description"));
    let prompt_original = Some(js::slice_chars(js::gs(raw, "promptOriginal").trim(), 600))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| js::slice_chars(fallback_prompt.trim(), 600));

    let moteur = match js::gs(raw, "moteur").trim().to_lowercase().as_str() {
        "agy" => "agy",
        "claude" => "claude",
        _ => "auto",
    };
    let modele = sanitize_model(moteur, &js::gs(raw, "modele"));

    let d_raw = &raw["declencheurs"];
    let d_base = &base["declencheurs"];

    let h_raw = &d_raw["horaire"];
    let h_base = &d_base["horaire"];
    let h_actif = h_raw["actif"].as_bool().unwrap_or_else(|| js::gb(h_base, "actif"));
    let h_type = match js::gs(h_raw, "type").as_str() {
        "intervalle" => "intervalle",
        "quotidien" => "quotidien",
        _ => if js::gs(h_base, "type") == "intervalle" { "intervalle" } else { "quotidien" },
    };
    let h_heure = Some(js::gs(h_raw, "heure"))
        .filter(|h| crate::re!(r"^\d{2}:\d{2}$").is_match(h))
        .unwrap_or_else(|| js::gs(h_base, "heure"));
    let h_minutes = js::gn(h_raw, "minutes")
        .map(|n| (n as i64).clamp(5, 1440))
        .unwrap_or_else(|| js::gn_or(h_base, "minutes", 60.0) as i64);
    let h_jours: Vec<i64> = h_raw["jours"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_i64()).filter(|d| (1..=7).contains(d)).collect())
        .filter(|v: &Vec<i64>| !v.is_empty())
        .unwrap_or_else(|| vec![1, 2, 3, 4, 5, 6, 7]);

    let m_raw = &d_raw["materiel"];
    let m_base = &d_base["materiel"];
    let m_actif = m_raw["actif"].as_bool().unwrap_or_else(|| js::gb(m_base, "actif"));
    let cpu_pct = js::gn(m_raw, "cpuPct").map(|n| (n as i64).clamp(30, 99)).unwrap_or_else(|| js::gn_or(m_base, "cpuPct", 85.0) as i64);
    let ram_pct = js::gn(m_raw, "ramPct").map(|n| (n as i64).clamp(30, 99)).unwrap_or_else(|| js::gn_or(m_base, "ramPct", 85.0) as i64);
    let gpu_pct = js::gn(m_raw, "gpuPct").map(|n| (n as i64).clamp(30, 99)).unwrap_or_else(|| js::gn_or(m_base, "gpuPct", 85.0) as i64);
    let gpu_temp = js::gn(m_raw, "gpuTemp").map(|n| (n as i64).clamp(50, 98)).unwrap_or_else(|| js::gn_or(m_base, "gpuTemp", 83.0) as i64);
    let duree_s = js::gn(m_raw, "dureeSecondes").map(|n| (n as i64).clamp(5, 300)).unwrap_or(15);
    let cooldown_m = js::gn(m_raw, "cooldownMinutes").map(|n| (n as i64).clamp(2, 720)).unwrap_or(30);

    let ml_raw = &d_raw["mails"];
    let ml_base = &d_base["mails"];
    let ml_actif = ml_raw["actif"].as_bool().unwrap_or_else(|| js::gb(ml_base, "actif"));
    let ml_mode = match js::gs(ml_raw, "mode").as_str() {
        "nouveau_mail" => "nouveau_mail",
        _ => "resume_jour",
    };
    let ml_filtre = js::slice_chars(js::gs(ml_raw, "filtre").trim(), 100);

    let dm_raw = &d_raw["demande"];
    let dm_base = &d_base["demande"];
    let dm_actif = dm_raw["actif"].as_bool().unwrap_or_else(|| js::gb(dm_base, "actif"));
    let mut mots_cles: Vec<String> = if dm_raw["motsCles"].is_array() {
        js::str_list(&dm_raw["motsCles"])
    } else {
        js::str_list(&dm_base["motsCles"])
    }
    .into_iter()
    .map(|s| js::slice_chars(s.trim(), 50))
    .filter(|s| !s.is_empty())
    .take(10)
    .collect();
    mots_cles.dedup();

    let mut sources: Vec<String> = if raw["sources"].is_array() {
        js::str_list(&raw["sources"])
    } else {
        js::str_list(&base["sources"])
    }
    .into_iter()
    .filter(|s| ["mails", "materiel", "web", "agenda"].contains(&s.as_str()))
    .collect();
    if ml_actif && !sources.contains(&"mails".to_string()) {
        sources.push("mails".into());
    }
    if m_actif && !sources.contains(&"materiel".to_string()) {
        sources.push("materiel".into());
    }
    sources.dedup();

    let consigne_ia = Some(js::slice_chars(js::gs(raw, "consigneIa").trim(), 1200))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| js::gs(&base, "consigneIa"));

    let s_raw = &raw["sortie"];
    let s_mode = match js::gs(s_raw, "mode").as_str() {
        "visuel" => "visuel",
        "silencieux" => "silencieux",
        _ => "voix",
    };
    let s_telegram = js::gb(s_raw, "telegram");

    let actif = raw["actif"].as_bool().unwrap_or(true);
    let cree_le = Some(js::gs(raw, "creeLe")).filter(|s| !s.is_empty()).unwrap_or_else(js::iso_now);
    let historique: Vec<J> = js::arr(&raw["historique"]).into_iter().take(MAX_HISTORIQUE).collect();

    json!({
        "id": id,
        "nom": nom,
        "icone": icone,
        "description": description,
        "promptOriginal": prompt_original,
        "actif": actif,
        "moteur": moteur,
        "modele": modele,
        "declencheurs": {
            "horaire": {
                "actif": h_actif,
                "type": h_type,
                "heure": h_heure,
                "minutes": h_minutes,
                "jours": h_jours
            },
            "materiel": {
                "actif": m_actif,
                "cpuPct": cpu_pct,
                "ramPct": ram_pct,
                "gpuPct": gpu_pct,
                "gpuTemp": gpu_temp,
                "dureeSecondes": duree_s,
                "cooldownMinutes": cooldown_m
            },
            "mails": {
                "actif": ml_actif,
                "mode": ml_mode,
                "filtre": ml_filtre
            },
            "demande": {
                "actif": dm_actif,
                "motsCles": mots_cles
            }
        },
        "sources": sources,
        "consigneIa": consigne_ia,
        "sortie": {
            "mode": s_mode,
            "telegram": s_telegram
        },
        "creeLe": cree_le,
        "derniereExecution": raw.get("derniereExecution").cloned().unwrap_or(Value::Null),
        "dernierStatut": raw.get("dernierStatut").cloned().unwrap_or(Value::Null),
        "dernierDeclencheur": raw.get("dernierDeclencheur").cloned().unwrap_or(Value::Null),
        "dernierModele": raw.get("dernierModele").cloned().unwrap_or(Value::Null),
        "dernierResultat": js::gs(raw, "dernierResultat"),
        "dernierJourHoraire": js::gs(raw, "dernierJourHoraire"),
        "derniereExecutionHoraireMs": raw["derniereExecutionHoraireMs"].as_i64().unwrap_or(0),
        "nbExecutions": raw["nbExecutions"].as_i64().unwrap_or(0),
        "historique": historique
    })
}

/// Évalue si les métriques matérielles dépassent l'un des seuils configurés sur l'agent.
pub fn check_hardware_thresholds(materiel_cfg: &J, telem: &J) -> Option<String> {
    if !js::gb(materiel_cfg, "actif") {
        return None;
    }
    let cpu_limit = js::gn_or(materiel_cfg, "cpuPct", 85.0) as i64;
    let ram_limit = js::gn_or(materiel_cfg, "ramPct", 85.0) as i64;
    let gpu_limit = js::gn_or(materiel_cfg, "gpuPct", 85.0) as i64;
    let temp_limit = js::gn_or(materiel_cfg, "gpuTemp", 83.0) as i64;

    let cpu = js::gn_or(telem, "cpu", 0.0).round() as i64;
    let ram = js::gn_or(telem, "ram", 0.0).round() as i64;
    let gpu = js::gn_or(telem, "gpuLoad", 0.0).round() as i64;
    let gpu_temp = js::gn_or(telem, "gpuTemp", 0.0).round() as i64;

    let mut reasons: Vec<String> = vec![];
    if cpu >= cpu_limit {
        reasons.push(format!("CPU à {cpu} % (seuil {cpu_limit} %)"));
    }
    if ram >= ram_limit {
        reasons.push(format!("RAM à {ram} % (seuil {ram_limit} %)"));
    }
    if gpu >= gpu_limit {
        reasons.push(format!("GPU à {gpu} % (seuil {gpu_limit} %)"));
    }
    if gpu_temp >= temp_limit {
        reasons.push(format!("Température GPU à {gpu_temp} °C (seuil {temp_limit} °C)"));
    }

    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join(", "))
    }
}

pub struct Agents {
    o: OrchRef,
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    pub items: RefCell<Vec<J>>,
    timer: Timer,
    /// id d'agent → cours d'exécution
    running_ids: RefCell< std::collections::HashSet<String> >,
    /// id d'agent → (instant premier dépassement ms, dernier déclenchement matériel ms)
    hw_state: RefCell<HashMap<String, (i64, i64)>>,
    /// id d'agent → clés des mails déjà vus
    mail_seen: RefCell<HashMap<String, Vec<String>>>,
}

impl Agents {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let items: Vec<J> = js::read_json(&file)
            .map(|d| js::arr(&d["items"]).iter().map(|a| normalize_agent(a, "", None)).collect())
            .unwrap_or_default();
        Rc::new(Self {
            o,
            file,
            log,
            items: RefCell::new(items),
            timer: Timer::new(),
            running_ids: RefCell::new(Default::default()),
            hw_state: RefCell::new(HashMap::new()),
            mail_seen: RefCell::new(HashMap::new()),
        })
    }

    fn save(&self) {
        let data = json!({ "version": 1, "items": self.items.borrow().clone() });
        if let Err(e) = js::write_json_atomic(&self.file, &data) {
            (self.log)(format!("agents : sauvegarde impossible ({e})"));
        }
    }

    fn changed(&self, agent_id: Option<&str>) {
        if let Some(o) = self.o.upgrade() {
            o.emit(json!({
                "type": "agents_changed",
                "agentId": agent_id,
                "count": self.count()
            }));
        }
    }

    pub fn count(&self) -> usize {
        self.items.borrow().len()
    }

    pub fn list(&self) -> Vec<J> {
        let running = self.running_ids.borrow();
        self.items
            .borrow()
            .iter()
            .map(|a| {
                let mut item = a.clone();
                let id = js::gs(&item, "id");
                item["enCours"] = json!(running.contains(&id));
                let (eng, mdl, lbl) = self.resolve_engine_and_model(&item);
                item["moteurEffectif"] = json!(eng);
                item["modeleEffectif"] = json!(mdl);
                item["modeleLibelle"] = json!(lbl);
                item
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<J> {
        self.list().into_iter().find(|a| js::gs(a, "id") == id)
    }

    /// Résout le moteur (« agy » ou « claude ») et le modèle effectif (minimum 3.8 Flash pour AGY,
    /// minimum Haiku pour Claude), avec bascule automatique si le quota AGY est épuisé.
    pub fn resolve_engine_and_model(&self, agent: &J) -> (String, String, String) {
        let pref_engine = js::gs(agent, "moteur");
        let pref_model = js::gs(agent, "modele");

        let (active_engine, agy_ok, claude_ok) = match self.o.upgrade() {
            Some(o) => {
                let d = o.status.d.borrow();
                let eng = d.engine.clone();
                drop(d);
                (eng, !o.agy.is_empty() && !o.agy_blocked(), !o.claude.is_empty())
            }
            None => ("claude".into(), false, true),
        };

        let want_agy = match pref_engine.as_str() {
            "agy" => true,
            "claude" => false,
            _ => active_engine == "agy",
        };

        let engine = if want_agy && agy_ok {
            "agy"
        } else if !want_agy && claude_ok {
            "claude"
        } else if agy_ok {
            "agy"
        } else {
            "claude"
        };

        let model = if pref_model.is_empty() || pref_model == "auto" {
            if engine == "agy" {
                "gemini-3.8-flash-high".to_string()
            } else {
                "haiku".to_string()
            }
        } else {
            sanitize_model(engine, &pref_model)
        };

        let label = model_label(engine, &model);
        (engine.to_string(), model, label)
    }

    /// État complet renvoyé au module UI `modules/agents`.
    pub fn state(&self) -> J {
        let (moteur_actif, modele_actif, tg_ok) = match self.o.upgrade() {
            Some(o) => {
                let d = o.status.d.borrow();
                let eng = d.engine.clone();
                drop(d);
                let fake = json!({ "moteur": "auto", "modele": "auto" });
                let (_, _, lbl) = self.resolve_engine_and_model(&fake);
                let tg = o.telegram.state();
                (eng, lbl, js::gb(&tg, "configured") && js::gb(&tg, "paired"))
            }
            None => ("claude".into(), "Claude · Haiku 4.5".into(), false),
        };

        let mut modeles = vec![json!({ "id": "auto", "moteur": "auto", "label": format!("Automatique ({modele_actif})") })];
        for (id, label) in AGY_MODELS {
            modeles.push(json!({ "id": id, "moteur": "agy", "label": label }));
        }
        for (id, label) in CLAUDE_MODELS {
            modeles.push(json!({ "id": id, "moteur": "claude", "label": label }));
        }

        json!({
            "ok": true,
            "agents": self.list(),
            "count": self.count(),
            "moteurActif": moteur_actif,
            "modeleActif": modele_actif,
            "modeles": modeles,
            "telegramConfigure": tg_ok
        })
    }

    pub fn start(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        self.timer.every(TICK_MS, move || {
            if let Some(agents) = me.upgrade() {
                let a = agents.clone();
                js::spawn(async move {
                    a.tick().await;
                });
            }
        });
    }

    pub fn stop(&self) {
        self.timer.clear();
    }

    /// Boucle de surveillance périodique (toutes les 10 s) :
    /// 1. Capteur matériel (CPU / RAM / GPU) — 0 jeton tant que les seuils ne sont pas dépassés.
    /// 2. Capteur horaire (quotidien à heure fixe ou intervalle).
    /// 3. Capteur de nouveaux mails.
    pub async fn tick(self: &Rc<Self>) {
        let Some(o) = self.o.upgrade() else { return };
        let now = js::now_ms();
        let local_now = Local::now();
        let today = local_now.format("%Y-%m-%d").to_string();
        let hm = format!("{:02}:{:02}", local_now.hour(), local_now.minute());
        let weekday = local_now.weekday().number_from_monday() as i64; // 1 = lundi .. 7 = dimanche

        // Relevé télémétrie locale (0 token)
        let mut telem = o.last_telemetry.borrow().clone();
        if telem.is_null() {
            telem = super::system::telemetry();
        }
        if let Some(probe) = o.hardware_probe.borrow().as_ref() {
            js::assign(&mut telem, probe.telemetry());
        }

        let snapshot: Vec<J> = self.items.borrow().clone();
        let mut to_run: Vec<(String, String)> = vec![];

        for agent in &snapshot {
            if !js::gb(agent, "actif") {
                continue;
            }
            let id = js::gs(agent, "id");
            if self.running_ids.borrow().contains(&id) {
                continue;
            }
            let decl = &agent["declencheurs"];

            // 1. Surveillance matérielle (CPU, RAM, GPU)
            let mat = &decl["materiel"];
            if js::gb(mat, "actif") {
                if let Some(reason) = check_hardware_thresholds(mat, &telem) {
                    let duree_ms = (js::gn_or(mat, "dureeSecondes", 15.0) as i64).max(5) * 1000;
                    let cooldown_ms = (js::gn_or(mat, "cooldownMinutes", 30.0) as i64).max(2) * 60_000;
                    let mut hw = self.hw_state.borrow_mut();
                    let (first_seen, last_fired) = hw.get(&id).copied().unwrap_or((0, 0));
                    let first = if first_seen == 0 { now } else { first_seen };
                    hw.insert(id.clone(), (first, last_fired));
                    if now - first >= duree_ms && (last_fired == 0 || now - last_fired >= cooldown_ms) {
                        hw.insert(id.clone(), (0, now));
                        to_run.push((id.clone(), format!("Alerte matériel : {reason}")));
                        continue;
                    }
                } else {
                    self.hw_state.borrow_mut().entry(id.clone()).and_modify(|e| e.0 = 0);
                }
            }

            // 2. Surveillance horaire (quotidien ou intervalle)
            let hor = &decl["horaire"];
            if js::gb(hor, "actif") {
                let jours: Vec<i64> = hor["jours"].as_array().map(|a| a.iter().filter_map(|v| v.as_i64()).collect()).unwrap_or_else(|| vec![1, 2, 3, 4, 5, 6, 7]);
                if jours.contains(&weekday) {
                    let h_type = js::gs(hor, "type");
                    if h_type == "intervalle" {
                        let every_ms = (js::gn_or(hor, "minutes", 60.0) as i64).max(5) * 60_000;
                        let last_ms = agent["derniereExecutionHoraireMs"].as_i64().unwrap_or(0);
                        if last_ms == 0 || now - last_ms >= every_ms {
                            self.mark_schedule_run(&id, &today, now);
                            to_run.push((id.clone(), format!("Programmation toutes les {} min", every_ms / 60_000)));
                            continue;
                        }
                    } else {
                        let target_hm = js::gs(hor, "heure");
                        let last_day = js::gs(agent, "dernierJourHoraire");
                        if !target_hm.is_empty() && hm >= target_hm && last_day != today {
                            self.mark_schedule_run(&id, &today, now);
                            to_run.push((id.clone(), format!("Exécution quotidienne ({target_hm})")));
                            continue;
                        }
                    }
                }
            }

            // 3. Surveillance de nouveaux mails en continu (si mode == "nouveau_mail")
            let mcfg = &decl["mails"];
            if js::gb(mcfg, "actif") && js::gs(mcfg, "mode") == "nouveau_mail" && o.google.is_configured() {
                let last_ms = agent["derniereExecutionHoraireMs"].as_i64().unwrap_or(0);
                if now - last_ms >= 3 * 60_000 {
                    self.mark_schedule_run(&id, &today, now);
                    if let Some(mail_reason) = self.check_new_mails_for_agent(&id, &js::gs(mcfg, "filtre")).await {
                        to_run.push((id.clone(), mail_reason));
                    }
                }
            }
        }

        for (id, trigger_reason) in to_run {
            let me = self.clone();
            js::spawn(async move {
                let _ = me.execute_agent(&id, &trigger_reason).await;
            });
        }
    }

    fn mark_schedule_run(&self, id: &str, today: &str, now_ms: i64) {
        let mut items = self.items.borrow_mut();
        if let Some(a) = items.iter_mut().find(|a| js::gs(a, "id") == id) {
            a["dernierJourHoraire"] = json!(today);
            a["derniereExecutionHoraireMs"] = json!(now_ms);
        }
        drop(items);
        self.save();
    }

    async fn check_new_mails_for_agent(&self, agent_id: &str, filtre: &str) -> Option<String> {
        let o = self.o.upgrade()?;
        let r = o.google.mail_derniers(&json!({ "limite": 8, "tous": false })).await;
        if !js::gb(&r, "ok") {
            return None;
        }
        let mails = js::arr(&r["mails"]);
        let filter_fold = fold(filtre);
        let matching: Vec<J> = mails
            .into_iter()
            .filter(|m| {
                if filter_fold.is_empty() {
                    return true;
                }
                let hay = fold(&format!("{} {}", js::gs(m, "from"), js::gs(m, "subject")));
                hay.contains(&filter_fold)
            })
            .collect();
        let keys: Vec<String> = matching.iter().map(|m| format!("{}|{}|{}", js::gs(m, "from"), js::gs(m, "subject"), js::gs(m, "date"))).collect();
        let mut seen_map = self.mail_seen.borrow_mut();
        let Some(known) = seen_map.get_mut(agent_id) else {
            seen_map.insert(agent_id.to_string(), keys);
            return None;
        };
        let fresh: Vec<&J> = matching
            .iter()
            .zip(keys.iter())
            .filter(|(_, k)| !known.contains(k))
            .map(|(m, _)| m)
            .collect();
        for k in keys {
            if !known.contains(&k) {
                known.push(k);
            }
        }
        if fresh.is_empty() {
            None
        } else {
            Some(format!("{} nouveau(x) mail(s) détecté(s)", fresh.len()))
        }
    }

    /// Crée un agent à partir d'une demande en langage naturel (zone de texte ou voix).
    pub async fn create_from_prompt(&self, prompt: &str, options: &J) -> J {
        let clean = prompt.trim();
        if clean.chars().count() < 4 {
            return json!({ "ok": false, "error": "Décris en quelques mots ce que l'agent doit surveiller et faire." });
        }
        if self.count() >= MAX_AGENTS {
            return json!({ "ok": false, "error": format!("Limite de {MAX_AGENTS} agents atteinte. Supprime un agent existant d'abord.") });
        }

        // 1. Analyse locale immédiate + enrichissement par l'IA (3.8 Flash / Haiku) si disponible
        let mut draft = parse_prompt_local(clean);
        let temp_agent = json!({
            "moteur": options.get("moteur").cloned().unwrap_or(json!("auto")),
            "modele": options.get("modele").cloned().unwrap_or(json!("auto"))
        });
        if let Some(ai_json) = self.call_ai_json(PARSE_SYSTEM_PROMPT, clean, &temp_agent, "Création d'agent").await {
            draft = js::spread(&draft, &ai_json);
        }

        draft["promptOriginal"] = json!(clean);
        if js::truthy(&options["moteur"]) {
            draft["moteur"] = options["moteur"].clone();
        }
        if js::truthy(&options["modele"]) {
            draft["modele"] = options["modele"].clone();
        }
        if options["sortie"].is_object() {
            draft["sortie"] = js::spread(&json!({ "mode": "voix", "telegram": false }), &options["sortie"]);
        }

        let agent = normalize_agent(&draft, clean, None);
        let id = js::gs(&agent, "id");
        self.items.borrow_mut().insert(0, agent.clone());
        self.save();

        if let Some(o) = self.o.upgrade() {
            o.record("agent_cree", json!({ "id": id, "nom": agent["nom"], "demande": js::slice_chars(clean, 200) }));
        }
        self.changed(Some(&id));
        json!({ "ok": true, "agent": self.get(&id).unwrap_or(agent) })
    }

    /// Modifie un agent existant à partir d'un formulaire structuré (`patch`).
    pub fn update(&self, id: &str, patch: &J) -> J {
        let mut items = self.items.borrow_mut();
        let Some(pos) = items.iter().position(|a| js::gs(a, "id") == id) else {
            return json!({ "ok": false, "error": "Agent introuvable." });
        };
        let current = items[pos].clone();
        let mut merged = js::spread(&current, patch);
        if patch["declencheurs"].is_object() {
            let mut decl = current["declencheurs"].clone();
            for key in ["horaire", "materiel", "mails", "demande"] {
                if patch["declencheurs"][key].is_object() {
                    decl[key] = js::spread(&current["declencheurs"][key], &patch["declencheurs"][key]);
                }
            }
            merged["declencheurs"] = decl;
        }
        if patch["sortie"].is_object() {
            merged["sortie"] = js::spread(&current["sortie"], &patch["sortie"]);
        }
        let prompt_orig = js::gs(&merged, "promptOriginal");
        let updated = normalize_agent(&merged, &prompt_orig, Some(id));
        items[pos] = updated.clone();
        drop(items);
        self.save();
        if let Some(o) = self.o.upgrade() {
            o.record("agent_modifie", json!({ "id": id, "nom": updated["nom"] }));
        }
        self.changed(Some(id));
        json!({ "ok": true, "agent": self.get(id).unwrap_or(updated) })
    }

    /// Modifie un agent en réinterprétant une nouvelle demande en langage naturel.
    pub async fn update_from_prompt(&self, id: &str, prompt: &str) -> J {
        let clean = prompt.trim();
        if clean.chars().count() < 4 {
            return json!({ "ok": false, "error": "La demande est trop courte." });
        }
        let Some(current) = self.get(id) else {
            return json!({ "ok": false, "error": "Agent introuvable." });
        };
        let mut draft = parse_prompt_local(clean);
        let input = format!(
            "Agent actuel :\n{}\n\nNouvelle demande de l'utilisateur :\n{clean}",
            serde_json::to_string(&current).unwrap_or_default()
        );
        if let Some(ai_json) = self.call_ai_json(PARSE_SYSTEM_PROMPT, &input, &current, "Modification d'agent").await {
            draft = js::spread(&draft, &ai_json);
        }
        draft["promptOriginal"] = json!(clean);
        self.update(id, &draft)
    }

    pub fn toggle(&self, id: &str, actif: Option<bool>) -> J {
        let Some(cur) = self.get(id) else {
            return json!({ "ok": false, "error": "Agent introuvable." });
        };
        let next = actif.unwrap_or(!js::gb(&cur, "actif"));
        self.update(id, &json!({ "actif": next }))
    }

    pub fn delete(&self, id: &str) -> J {
        let mut items = self.items.borrow_mut();
        let Some(pos) = items.iter().position(|a| js::gs(a, "id") == id) else {
            return json!({ "ok": false, "error": "Agent introuvable." });
        };
        let removed = items.remove(pos);
        drop(items);
        self.hw_state.borrow_mut().remove(id);
        self.mail_seen.borrow_mut().remove(id);
        self.save();
        if let Some(o) = self.o.upgrade() {
            o.record("agent_supprime", json!({ "id": id, "nom": removed["nom"] }));
        }
        self.changed(Some(id));
        json!({ "ok": true, "deleted": id, "nom": removed["nom"] })
    }

    pub fn clear_history(&self, id: &str) -> J {
        let mut items = self.items.borrow_mut();
        let Some(agent) = items.iter_mut().find(|a| js::gs(a, "id") == id) else {
            return json!({ "ok": false, "error": "Agent introuvable." });
        };
        agent["historique"] = json!([]);
        agent["dernierResultat"] = json!("");
        agent["dernierStatut"] = Value::Null;
        drop(items);
        self.save();
        self.changed(Some(id));
        json!({ "ok": true, "agent": self.get(id) })
    }

    /// Collecte le contexte temps réel (mails, matériel + processus gourmands, agenda, date/heure)
    /// avant d'interroger l'IA.
    async fn gather_context(&self, agent: &J, trigger_reason: &str) -> String {
        let mut sections: Vec<String> = vec![
            format!("Date et heure actuelles : {}", Local::now().format("%A %d/%m/%Y à %H:%M")),
            format!("Motif du déclenchement : {trigger_reason}"),
        ];
        let Some(o) = self.o.upgrade() else {
            return sections.join("\n\n");
        };
        let sources = js::str_list(&agent["sources"]);

        // 1. Source : Mails
        if sources.iter().any(|s| s == "mails") {
            if o.google.is_configured() {
                let r = o.google.mail_derniers(&json!({ "limite": 12, "tous": true })).await;
                if js::gb(&r, "ok") {
                    let mails = js::arr(&r["mails"]);
                    if mails.is_empty() {
                        sections.push("Boîte Gmail : aucun mail récent trouvé.".into());
                    } else {
                        let lines: Vec<String> = mails
                            .iter()
                            .enumerate()
                            .map(|(i, m)| {
                                format!(
                                    "{}. De: {} | Objet: {} | Date: {} | Extrait: {}",
                                    i + 1,
                                    js::gs(m, "from"),
                                    js::gs(m, "subject"),
                                    js::gs(m, "date"),
                                    js::slice_chars(&js::gs(m, "snippet"), 180)
                                )
                            })
                            .collect();
                        sections.push(format!("Derniers mails reçus ({}) :\n{}", mails.len(), lines.join("\n")));
                    }
                } else {
                    sections.push(format!("Boîte Gmail : erreur de lecture ({}).", js::gs(&r, "message")));
                }
            } else {
                sections.push("Boîte Gmail : le connecteur Gmail n'est pas encore configuré dans l'Administration.".into());
            }
        }

        // 2. Source : Matériel (CPU, RAM, GPU, processus les plus gourmands)
        if sources.iter().any(|s| s == "materiel") {
            let mut telem = o.last_telemetry.borrow().clone();
            if telem.is_null() {
                telem = super::system::telemetry();
            }
            if let Some(probe) = o.hardware_probe.borrow().as_ref() {
                js::assign(&mut telem, probe.telemetry());
            }
            let cpu = js::gn_or(&telem, "cpu", 0.0).round() as i64;
            let ram = js::gn_or(&telem, "ram", 0.0).round() as i64;
            let ram_used = js::gn_or(&telem, "ramUsedGb", 0.0);
            let ram_total = js::gn_or(&telem, "ramTotalGb", 0.0);
            let gpu = js::gn(&telem, "gpuLoad").map(|v| format!("{} %", v.round() as i64)).unwrap_or_else(|| "non mesuré".into());
            let gpu_temp = js::gn(&telem, "gpuTemp").map(|v| format!("{} °C", v.round() as i64)).unwrap_or_else(|| "non mesurée".into());
            let gpu_name = js::gs(&telem, "gpuName");

            let top_procs = Self::top_processes().await;
            sections.push(format!(
                "État matériel du PC en temps réel :\n- CPU : {cpu} %\n- RAM : {ram} % ({ram_used:.1} Go / {ram_total:.1} Go)\n- GPU {} : charge {gpu}, température {gpu_temp}\n- Processus les plus gourmands :\n{top_procs}",
                if gpu_name.is_empty() { String::new() } else { format!("({gpu_name})") }
            ));
        }

        // 3. Source : Agenda
        if sources.iter().any(|s| s == "agenda") && o.google.is_configured() {
            let r = o.google.agenda_semaine(&json!({})).await;
            if js::gb(&r, "ok") {
                let evs = js::arr(&r["evenements"]);
                let lines: Vec<String> = evs
                    .iter()
                    .take(10)
                    .map(|e| format!("- {} {} : {}", js::gs(e, "date"), js::gs(e, "heure"), js::gs(e, "titre")))
                    .collect();
                sections.push(format!("Prochains rendez-vous de l'agenda :\n{}", if lines.is_empty() { "Aucun rendez-vous prévu.".into() } else { lines.join("\n") }));
            }
        }

        sections.join("\n\n")
    }

    async fn top_processes() -> String {
        if js::env("JARVIS_TEST").is_some() {
            return "  • chrome.exe : 1 420 Mo RAM\n  • code.exe : 890 Mo RAM\n  • rust-analyzer.exe : 640 Mo RAM".into();
        }
        let args: Vec<String> = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            "Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 6 Name, @{N='RAM_Mo';E={[math]::Round($_.WorkingSet64/1MB)}}, @{N='CPU_s';E={[math]::Round($_.CPU,1)}} | ForEach-Object { \"  • $($_.Name) : $($_.RAM_Mo) Mo RAM (CPU cumulé: $($_.CPU_s)s)\" }".into(),
        ];
        match crate::proc::run("powershell.exe", &args, None, 6000).await {
            Ok(o) if o.code == 0 && !o.stdout.trim().is_empty() => o.stdout.trim().to_string(),
            _ => "  (liste des processus indisponible)".into(),
        }
    }

    /// Exécute un agent (manuellement, sur horaire, sur alerte matérielle ou à la voix).
    pub async fn execute_agent(self: &Rc<Self>, id: &str, trigger_reason: &str) -> J {
        let Some(agent) = self.get(id) else {
            return json!({ "ok": false, "error": "Agent introuvable." });
        };
        if self.running_ids.borrow().contains(id) {
            return json!({ "ok": false, "error": "Cet agent est déjà en cours d'exécution." });
        }

        self.running_ids.borrow_mut().insert(id.to_string());
        self.changed(Some(id));

        let nom = js::gs(&agent, "nom");
        let icone = js::gs(&agent, "icone");
        let consigne = js::gs(&agent, "consigneIa");
        let needs_web = js::str_list(&agent["sources"]).iter().any(|s| s == "web");
        let context = self.gather_context(&agent, trigger_reason).await;

        let system_prompt = format!(
            "Tu es l'agent autonome « {nom} » intégré à Jarvis. \
            Réponds en français, de manière directe, utile et bien structurée. \
            Commence par une synthèse claire de 1 à 3 phrases courtes (qui pourra être lue à voix haute sans markdown lourd), \
            puis donne les détails ou conseils pratiques ligne par ligne avec des puces « • »."
        );
        let user_input = format!(
            "Mission de l'agent :\n{consigne}\n\nContexte relevé par Jarvis :\n{context}{}",
            if needs_web {
                "\n\nUtilise la recherche web (WebSearch / WebFetch) pour récupérer les informations à jour (dates, horaires, chaînes de diffusion, actualités) avant de répondre."
            } else {
                ""
            }
        );

        let (engine, model, model_lbl) = self.resolve_engine_and_model(&agent);
        let ai_res = self.run_ai(&system_prompt, &user_input, &engine, &model, needs_web, &format!("Agent · {nom}")).await;

        let (ok, answer, used_label) = match ai_res {
            Ok((txt, lbl)) => (true, txt, lbl),
            Err(err_msg) => {
                // Repli local synthétique si le binaire IA est absent (ou en environnement de test)
                let fallback = Self::fallback_execution_report(&agent, &context, trigger_reason, &err_msg);
                (true, fallback, format!("{model_lbl} (synthèse locale)"))
            }
        };

        self.running_ids.borrow_mut().remove(id);
        let now_iso = js::iso_now();
        let entry = json!({
            "at": now_iso,
            "declencheur": trigger_reason,
            "modele": used_label,
            "statut": if ok { "ok" } else { "erreur" },
            "resultat": js::slice_chars(&answer, 4000)
        });

        {
            let mut items = self.items.borrow_mut();
            if let Some(a) = items.iter_mut().find(|a| js::gs(a, "id") == id) {
                a["derniereExecution"] = json!(now_iso);
                a["dernierStatut"] = json!(if ok { "ok" } else { "erreur" });
                a["dernierDeclencheur"] = json!(trigger_reason);
                a["dernierModele"] = json!(used_label);
                a["dernierResultat"] = json!(js::slice_chars(&answer, 4000));
                a["nbExecutions"] = json!(a["nbExecutions"].as_i64().unwrap_or(0) + 1);
                let mut hist = js::arr(&a["historique"]);
                hist.insert(0, entry.clone());
                hist.truncate(MAX_HISTORIQUE);
                a["historique"] = Value::Array(hist);
            }
        }
        self.save();
        self.changed(Some(id));

        // Restitution selon le canal configuré sur l'agent (voix / visuel / silencieux + Telegram)
        if let Some(o) = self.o.upgrade() {
            o.record(
                "agent_execution",
                json!({
                    "id": id,
                    "nom": nom,
                    "declencheur": trigger_reason,
                    "modele": used_label,
                    "statut": if ok { "ok" } else { "erreur" }
                }),
            );
            self.deliver_output(&o, &agent, &icone, &nom, trigger_reason, &used_label, &answer);
        }

        json!({
            "ok": ok,
            "agent": self.get(id),
            "execution": entry
        })
    }

    fn deliver_output(&self, o: & crate::orch::Orchestrator, agent: &J, icone: &str, nom: &str, trigger_reason: &str, used_label: &str, answer: &str) {
        let mode = js::gs(&agent["sortie"], "mode");
        let send_tg = js::gb(&agent["sortie"], "telegram");

        // Découpe la réponse en lignes pour la card HUD
        let mut rows: Vec<J> = answer
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .take(10)
            .map(|line| {
                let clean_line = crate::re!(r"^[•\-*]\s*").replace(line, "").to_string();
                json!({
                    "icon": icone,
                    "title": js::slice_chars(&clean_line, 140)
                })
            })
            .collect();
        if rows.is_empty() {
            rows.push(json!({ "icon": icone, "title": js::slice_chars(answer, 140) }));
        }

        let spoken_summary = super::speakable(
            answer
                .lines()
                .filter(|l| !l.trim().is_empty())
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
                .as_str(),
        );

        if mode != "silencieux" {
            let speak = mode == "voix";
            o.notify(json!({
                "title": format!("{icone} {nom}"),
                "text": format!("{nom} : {spoken_summary}"),
                "source": "agent",
                "speak": speak,
                "card": true,
                "rows": rows,
                "footer": format!("{trigger_reason} · {used_label}")
            }));
        }

        if send_tg {
            o.telegram.send(&format!("{icone} Agent {nom} ({trigger_reason})\n\n{}", js::slice_chars(answer, 3500)), None);
        }
    }

    fn fallback_execution_report(agent: &J, context: &str, trigger_reason: &str, _err: &str) -> String {
        let nom = js::gs(agent, "nom");
        let sources = js::str_list(&agent["sources"]);
        if sources.iter().any(|s| s == "materiel") {
            format!(
                "Rapport de surveillance matérielle ({trigger_reason}).\n\
                • Conseils d'optimisation : fermez les onglets navigateur inutilisés ou les processus lourds listés ci-dessous, vérifiez les tâches de compilation en arrière-plan et assurez-vous que le mode d'alimentation Windows est adapté.\n\
                • Relevé système :\n{context}"
            )
        } else if sources.iter().any(|s| s == "mails") {
            format!(
                "Résumé automatique des mails ({trigger_reason}).\n\
                • Synthèse de votre boîte de réception :\n{context}"
            )
        } else {
            format!(
                "Exécution de l'agent « {nom} » ({trigger_reason}).\n\
                • Consigne : {}\n• Contexte : {context}",
                js::gs(agent, "consigneIa")
            )
        }
    }

    /// Appel IA JSON court (création / modification d'agent).
    async fn call_ai_json(&self, system: &str, input: &str, agent_hint: &J, label: &str) -> Option<J> {
        if js::env("JARVIS_TEST").is_some() {
            return None;
        }
        let (engine, model, _) = self.resolve_engine_and_model(agent_hint);
        let (text, _) = self.run_ai(system, input, &engine, &model, false, label).await.ok()?;
        super::shopping::read_offers_data(std::path::Path::new(""), &text)
    }

    /// Exécute un appel IA avec le moteur et le modèle résolus (AGY >= 3.8 Flash ou Claude >= Haiku).
    /// Bascule automatiquement sur l'autre moteur si le premier échoue ou si son quota est épuisé.
    async fn run_ai(&self, system: &str, input: &str, engine: &str, model: &str, needs_web: bool, label: &str) -> Result<(String, String), String> {
        if js::env("JARVIS_TEST").is_some() {
            return Err("mode test".into());
        }
        let Some(o) = self.o.upgrade() else {
            return Err("orchestrateur arrêté".into());
        };

        if engine == "agy" {
            match self.spawn_agy(&o, system, input, model, label).await {
                Ok(ans) => return Ok((ans, model_label("agy", model))),
                Err(e) => {
                    o.note_engine_error("agy", &e);
                    if !o.claude.is_empty() {
                        (self.log)(format!("agents : repli d'AGY ({e}) vers Claude Haiku"));
                        let ans = self.spawn_claude(&o, system, input, "haiku", needs_web, label).await?;
                        return Ok((ans, model_label("claude", "haiku")));
                    }
                    return Err(e);
                }
            }
        }

        match self.spawn_claude(&o, system, input, model, needs_web, label).await {
            Ok(ans) => Ok((ans, model_label("claude", model))),
            Err(e) => {
                if !o.agy.is_empty() && !o.agy_blocked() {
                    (self.log)(format!("agents : repli de Claude ({e}) vers AGY Gemini 3.8 Flash"));
                    let ans = self.spawn_agy(&o, system, input, "gemini-3.8-flash-high", label).await?;
                    return Ok((ans, model_label("agy", "gemini-3.8-flash-high")));
                }
                Err(e)
            }
        }
    }

    async fn spawn_claude(&self, o: &crate::orch::Orchestrator, system: &str, input: &str, model: &str, needs_web: bool, label: &str) -> Result<String, String> {
        let Some(claude) = o.claude_path() else {
            return Err("Claude Code introuvable".into());
        };
        let safe_model = sanitize_model("claude", model);
        let safe_model = if safe_model == "auto" { "haiku".to_string() } else { safe_model };
        let task_id = format!("ag_{}", js::now_ms());
        let lbl = model_label("claude", &safe_model);
        o.emit(json!({ "type": "create", "id": task_id, "label": label, "detail": lbl, "background": true }));
        o.start_ai_call(&task_id, "claude", &lbl, input, true);

        let tools = if needs_web { "WebSearch,WebFetch" } else { "" };
        use tokio::io::AsyncWriteExt;
        let mut cmd = tokio::process::Command::new(&claude);
        cmd.args([
            "-p",
            "--output-format",
            "text",
            "--tools",
            tools,
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--permission-mode",
            "manual",
            "--no-session-persistence",
            "--system-prompt",
            system,
            "--model",
            &safe_model,
        ])
        .current_dir(crate::paths::workspace())
        .env("JARVIS_TASK_ID", &task_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let timeout_s = if needs_web { 120 } else { 75 };
        let res: Result<String, String> = async {
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(input.as_bytes()).await;
            }
            let out = tokio::time::timeout(std::time::Duration::from_secs(timeout_s), child.wait_with_output())
                .await
                .map_err(|_| "délai dépassé".to_string())?
                .map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if out.status.success() && !stdout.is_empty() {
                Ok(stdout)
            } else if !stderr.is_empty() {
                Err(stderr)
            } else {
                Err("réponse vide".into())
            }
        }
        .await;

        let ans_ref = res.as_deref().unwrap_or("");
        o.end_ai_call(&task_id, ans_ref);
        o.emit(json!({ "type": "finish", "id": task_id, "status": if res.is_ok() { "done" } else { "error" } }));
        res
    }

    async fn spawn_agy(&self, o: &crate::orch::Orchestrator, system: &str, input: &str, model: &str, label: &str) -> Result<String, String> {
        if o.agy.is_empty() || (!std::path::Path::new(&o.agy).exists() && o.agy != "agy") {
            return Err("AGY introuvable".into());
        }
        let safe_model = sanitize_model("agy", model);
        let safe_model = if safe_model == "auto" { "gemini-3.8-flash-high".to_string() } else { safe_model };
        let task_id = format!("ag_{}", js::now_ms());
        let lbl = model_label("agy", &safe_model);
        o.emit(json!({ "type": "create", "id": task_id, "label": label, "detail": lbl, "background": true }));
        let full_prompt = format!("[Consignes : {system}]\n\n{input}");
        o.start_ai_call(&task_id, "agy", &lbl, &full_prompt, true);

        use tokio::io::AsyncWriteExt;
        let agy_effort = if safe_model.ends_with("-low") {
            "low"
        } else if safe_model.ends_with("-high") {
            "high"
        } else {
            "medium"
        };
        let mut cmd = tokio::process::Command::new(&o.agy);
        cmd.args([
            "--output-format",
            "stream-json",
            "--mode",
            "plan",
            "--add-dir",
            &o.workspace.display().to_string(),
            "--model",
            &safe_model,
            "--effort",
            agy_effort,
        ])
        .current_dir(&o.workspace)
        .env("JARVIS_TASK_ID", &task_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let res: Result<String, String> = async {
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(full_prompt.as_bytes()).await;
            }
            let out = tokio::time::timeout(std::time::Duration::from_secs(110), child.wait_with_output())
                .await
                .map_err(|_| "délai dépassé".to_string())?
                .map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let mut answer = String::new();
            for line in stdout.lines() {
                if let Ok(evt) = serde_json::from_str::<Value>(line) {
                    if js::gs(&evt, "event") == "step_update" && js::gs(&evt["step_update"], "step_type") == "agent_response" {
                        answer.push_str(&js::gs(&evt["step_update"], "text_delta"));
                    } else if js::gs(&evt, "event") == "result" {
                        let resp = js::gs(&evt["result"], "response");
                        if !resp.trim().is_empty() {
                            answer = resp;
                        }
                    }
                }
            }
            if answer.trim().is_empty() && !stdout.trim().is_empty() && !stdout.trim_start().starts_with('{') {
                answer = stdout.trim().to_string();
            }
            if out.status.success() && !answer.trim().is_empty() {
                Ok(answer.trim().to_string())
            } else if !stderr.is_empty() {
                Err(stderr)
            } else {
                Err("réponse AGY vide".into())
            }
        }
        .await;

        let ans_ref = res.as_deref().unwrap_or("");
        o.end_ai_call(&task_id, ans_ref);
        o.emit(json!({ "type": "finish", "id": task_id, "status": if res.is_ok() { "done" } else { "error" } }));
        res
    }

    /// Intentions vocales et textuelles liées aux Agents :
    /// - « ouvre mes agents », « affiche les agents », « liste mes agents »
    /// - « crée un agent qui … »
    /// - « lance l'agent … », « exécute l'agent … »
    /// - ou déclenchement par mots-clés d'un agent actif (« donne-moi la liste des matchs du Top 14 »).
    pub fn match_(self: &Rc<Self>, t: &str, raw: &str) -> Option<Intent> {
        let o = self.o.clone();

        // 1. Ouvrir / lister les agents
        if crate::re!(r"^(?:(?:ouvre|ouvrez|ouvrir|affiche|affichez|afficher|montre|montrez|montrer|lance|lancez|lancer|demarre|active|liste|donne)(?: moi)? (?:mes |les |la |le )?(?:(?:card|cards|carte|cartes|carde|cardes|module|modules|widget|panneau|fiche) (?:de |des |d )?)?(?:mes )?agents?(?: autonomes?)?(?: \d{1,3})?|mes agents(?: autonomes?)?)$").is_match(t) {
            let me = self.clone();
            return Some(Intent::new("agents_open", "Agents", "ouverture", move || {
                let o = o.clone();
                let me = me.clone();
                async move {
                    o.up().emit(json!({ "type": "agents_open" }));
                    let n = me.count();
                    let say = if n == 0 {
                        "J'ouvre le module Agents. Tu n'as pas encore d'agent configuré.".to_string()
                    } else {
                        let actifs = me.items.borrow().iter().filter(|a| js::gb(a, "actif")).count();
                        format!("J'ouvre tes agents : {n} configuré{}, dont {actifs} actif{}.", if n > 1 { "s" } else { "" }, if actifs > 1 { "s" } else { "" })
                    };
                    json!({ "say": say, "display": format!("{n} agent(s)") })
                }
            }));
        }

        // 2. Créer un agent à la voix (« crée un agent qui surveille mes mails… »)
        if let Some(m) = crate::re!(r"(?i)^(?:jarvis[, ]+)?(?:cr[ée]e|ajoute|configure|mets en place)(?:-moi| moi)? un agent (?:qui |pour |charg[ée] de )?(.+)$").captures(raw.trim()) {
            let prompt = m[1].trim().to_string();
            if prompt.chars().count() >= 6 {
                let me = self.clone();
                return Some(Intent::new("agent_create", "Nouvel agent", &js::slice_chars(&prompt, 36), move || {
                    let o = o.clone();
                    let me = me.clone();
                    let prompt = prompt.clone();
                    async move {
                        let r = me.create_from_prompt(&prompt, &json!({})).await;
                        o.up().emit(json!({ "type": "agents_open" }));
                        if js::gb(&r, "ok") {
                            let nom = js::gs(&r["agent"], "nom");
                            json!({ "say": format!("J'ai créé et activé l'agent {nom}."), "display": nom })
                        } else {
                            json!({ "say": js::gs(&r, "error"), "display": "Échec création" })
                        }
                    }
                }));
            }
        }

        // 3. Lancer explicitement un agent par son nom (« lance l'agent rugby », « exécute l'agent mails »)
        if let Some(m) = crate::re!(r"^(?:lance|execute|demarre|declenche|teste)(?: moi)? l ?agent (.+)$").captures(t) {
            let target = m[1].trim().to_string();
            if let Some(found) = self.find_by_name(&target) {
                let id = js::gs(&found, "id");
                let nom = js::gs(&found, "nom");
                let me = self.clone();
                return Some(Intent::new("agent_run", "Agent", &nom.clone(), move || {
                    let me = me.clone();
                    let id = id.clone();
                    let nom = nom.clone();
                    async move {
                        let me2 = me.clone();
                        js::spawn(async move {
                            let _ = me2.execute_agent(&id, "Demande vocale").await;
                        });
                        json!({ "say": format!("Je lance l'agent {nom}."), "display": nom, "quiet": true })
                    }
                }));
            }
        }

        // 4. Déclenchement sur demande via les mots-clés d'un agent actif (ex: « liste des matchs du Top 14 »)
        let folded = fold(t);
        let matched = self.items.borrow().iter().find(|a| {
            if !js::gb(a, "actif") || !js::gb(&a["declencheurs"]["demande"], "actif") {
                return false;
            }
            js::str_list(&a["declencheurs"]["demande"]["motsCles"])
                .iter()
                .map(|kw| fold(kw))
                .filter(|kw| kw.len() >= 4)
                .any(|kw| folded.contains(&kw))
        }).cloned();

        if let Some(agent) = matched {
            let id = js::gs(&agent, "id");
            let nom = js::gs(&agent, "nom");
            let me = self.clone();
            return Some(Intent::new("agent_keyword", "Agent", &nom.clone(), move || {
                let me = me.clone();
                let id = id.clone();
                let nom = nom.clone();
                async move {
                    let me2 = me.clone();
                    js::spawn(async move {
                        let _ = me2.execute_agent(&id, "Sur demande").await;
                    });
                    json!({ "say": format!("L'agent {nom} s'en occupe et te répond dès que c'est prêt."), "display": nom, "quiet": true })
                }
            }));
        }

        None
    }

    pub fn find_by_name(&self, query: &str) -> Option<J> {
        let q = fold(query);
        if q.is_empty() {
            return None;
        }
        self.items
            .borrow()
            .iter()
            .find(|a| {
                let n = fold(&js::gs(a, "nom"));
                n == q || n.contains(&q) || q.contains(&n)
            })
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_trois_exemples_utilisateur_sont_correctement_analyses() {
        // Exemple 1 : surveillance et résumé quotidien des mails
        let m = parse_prompt_local("surveille mes mails et résume moi une fois par jours mes mails");
        assert!(js::gb(&m["declencheurs"]["horaire"], "actif"));
        assert!(js::gb(&m["declencheurs"]["mails"], "actif"));
        assert_eq!(js::gs(&m["declencheurs"]["mails"], "mode"), "resume_jour");
        assert!(js::str_list(&m["sources"]).contains(&"mails".to_string()));

        // Exemple 2 : surveillance PC RAM / CPU / GPU + conseils performances
        let pc = parse_prompt_local("surveille mon pc et dés que la RAM, le CPU ou le GPU sont trop utilisé indique le moi et propose moi des conseilles pour améliorer mes performances");
        assert!(js::gb(&pc["declencheurs"]["materiel"], "actif"));
        assert_eq!(js::gn_or(&pc["declencheurs"]["materiel"], "cpuPct", 0.0) as i64, 85);
        assert_eq!(js::gn_or(&pc["declencheurs"]["materiel"], "ramPct", 0.0) as i64, 85);
        assert_eq!(js::gn_or(&pc["declencheurs"]["materiel"], "gpuPct", 0.0) as i64, 85);
        assert!(js::str_list(&pc["sources"]).contains(&"materiel".to_string()));

        // Exemple 3 : Rugby TOP 14 chaque jour ou sur demande avec date, heure et chaîne
        let rugby = parse_prompt_local("j'adore le rugby alors indique moi chaque jour ou sur demande la liste des matchs de rugby du TOP 14 avec l'heure date, heure et chaine de diffusion");
        assert!(js::gb(&rugby["declencheurs"]["horaire"], "actif"));
        assert!(js::gb(&rugby["declencheurs"]["demande"], "actif"));
        assert!(js::str_list(&rugby["sources"]).contains(&"web".to_string()));
        assert!(js::str_list(&rugby["declencheurs"]["demande"]["motsCles"]).iter().any(|k| k.contains("top 14")));
    }

    #[test]
    fn les_modeles_respectent_le_minimum_3_8_flash_et_haiku() {
        assert_eq!(sanitize_model("agy", "gemini-3.6-flash"), "gemini-3.8-flash-high");
        assert_eq!(sanitize_model("agy", "gemini-3.7-flash"), "gemini-3.8-flash-high");
        assert_eq!(sanitize_model("agy", "gemini-3.8-flash-low"), "gemini-3.8-flash-low");
        assert_eq!(sanitize_model("agy", "gemini-3.8-flash-medium"), "gemini-3.8-flash-medium");
        assert_eq!(sanitize_model("agy", "gemini-3.8-flash-high"), "gemini-3.8-flash-high");
        assert_eq!(sanitize_model("claude", "haiku"), "haiku");
        assert_eq!(sanitize_model("claude", "sonnet"), "sonnet");
        assert_eq!(sanitize_model("claude", "inconnu"), "haiku");
    }

    #[test]
    fn le_capteur_materiel_detecte_les_depassements_cpu_ram_gpu() {
        let cfg = json!({ "actif": true, "cpuPct": 85, "ramPct": 85, "gpuPct": 90, "gpuTemp": 83 });
        assert!(check_hardware_thresholds(&cfg, &json!({ "cpu": 40, "ram": 60, "gpuLoad": 30 })).is_none());
        let alert = check_hardware_thresholds(&cfg, &json!({ "cpu": 92, "ram": 88, "gpuLoad": 50 })).unwrap();
        assert!(alert.contains("CPU à 92 %"));
        assert!(alert.contains("RAM à 88 %"));
    }
}
