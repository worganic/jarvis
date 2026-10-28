// État de session de Jarvis (orchestrator/status.js) : moteur (claude / agy), modèle, effort, voix,
// thème, apparence du cœur, consommation du quota. Conservé entre deux lancements (data/session.json).
// `parse` reconnaît les commandes de réglage dites ou tapées (« /moteur agy », « parle plus vite »…).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::usage_daily::UsageDaily;
use crate::js::{self, J};

pub fn engine_of(name: &str) -> Option<&'static str> {
    Some(match name {
        "claude" | "claude code" | "claudecode" => "claude",
        "agy" | "agi" | "hgy" | "a g y" | "a, g, y" | "ajy" | "aji" | "antigravity" | "gemini" => "agy",
        _ => return None,
    })
}

pub const CLAUDE_MODELS: &[&str] = &["opus", "sonnet", "haiku", "fable"];

/// Modèles AGY distincts (valeurs de AGY_MODELS en v3), pour le catalogue de l'administration.
pub const AGY_MODEL_KEYS: &[&str] = &[
    "gemini-3.8-flash-high", "gemini-3.8-flash-medium", "gemini-3.8-flash-low",
    "gemini-3.7-flash-high", "gemini-3.7-flash-medium", "gemini-3.7-flash-low",
    "gemini-3.6-flash-high", "gemini-3.6-flash-medium", "gemini-3.6-flash-low",
    "gemini-3.1-pro-high", "gemini-3.1-pro-low", "claude-sonnet-4-6", "claude-opus-4-6-thinking", "gpt-oss-120b-medium",
];

pub fn agy_model(name: &str) -> Option<&'static str> {
    Some(match name {
        "gemini-3.8-flash-high" => "gemini-3.8-flash-high",
        "gemini-3.8-flash-medium" => "gemini-3.8-flash-medium",
        "gemini-3.8-flash-low" => "gemini-3.8-flash-low",
        "gemini-3.7-flash-high" => "gemini-3.7-flash-high",
        "gemini-3.7-flash-medium" => "gemini-3.7-flash-medium",
        "gemini-3.7-flash-low" => "gemini-3.7-flash-low",
        "gemini-3.6-flash-high" => "gemini-3.6-flash-high",
        "gemini-3.6-flash-medium" => "gemini-3.6-flash-medium",
        "gemini-3.6-flash-low" => "gemini-3.6-flash-low",
        "gemini-3.1-pro-high" => "gemini-3.1-pro-high",
        "gemini-3.1-pro-low" => "gemini-3.1-pro-low",
        "claude-sonnet-4-6" => "claude-sonnet-4-6",
        "claude-opus-4-6-thinking" => "claude-opus-4-6-thinking",
        "gpt-oss-120b-medium" => "gpt-oss-120b-medium",
        "gemini-3.8-flash" => "gemini-3.8-flash-high",
        "gemini-3.7-flash" => "gemini-3.7-flash-high",
        "gemini-3.6-flash" => "gemini-3.6-flash-high",
        "gemini-3.1-pro" => "gemini-3.1-pro-high",
        "gemini-flash" => "gemini-3.8-flash-high",
        "gemini-pro" => "gemini-3.1-pro-high",
        "gemini" => "gemini-3.8-flash-high",
        "flash" => "gemini-3.8-flash-high",
        "pro" => "gemini-3.1-pro-high",
        _ => return None,
    })
}

pub fn is_agy_model_key(name: &str) -> bool {
    agy_model(name).is_some()
}

pub fn model_of(name: &str) -> Option<String> {
    match name {
        "auto" | "automatique" | "smart" => Some("auto".into()),
        n if CLAUDE_MODELS.contains(&n) => Some(n.into()),
        n => agy_model(n).map(String::from),
    }
}

fn effort_of(label: &str) -> Option<&'static str> {
    Some(match label {
        "faible" | "bas" | "basse" | "minimum" => "low",
        "moyen" | "moyenne" | "normal" | "normale" => "medium",
        "eleve" | "élevé" | "élevée" | "fort" | "forte" | "haut" | "haute" => "high",
        "tres eleve" | "très élevé" | "tres haute" => "xhigh",
        "maximum" | "maximale" | "max" => "max",
        _ => return None,
    })
}

pub const EFFORT_VALUES: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// (clé, id edge-tts, nom, genre, origine)
pub const VOICES: &[(&str, &str, &str, &str, &str)] = &[
    ("denise", "fr-FR-DeniseNeural", "Denise", "Féminin", "France"),
    ("henri", "fr-FR-HenriNeural", "Henri", "Masculin", "France"),
    ("eloise", "fr-FR-EloiseNeural", "Éloïse", "Féminin", "France"),
    ("sylvie", "fr-CA-SylvieNeural", "Sylvie", "Féminin", "Canada"),
    ("antoine", "fr-CA-AntoineNeural", "Antoine", "Masculin", "Canada"),
    ("charline", "fr-BE-CharlineNeural", "Charline", "Féminin", "Belgique"),
    ("ariane", "fr-CH-ArianeNeural", "Ariane", "Féminin", "Suisse"),
    // Multilingues : les noms anglais (Airbus Helicopters…) dits naturellement au milieu du français.
    ("vivienne", "fr-FR-VivienneMultilingualNeural", "Vivienne (multilingue)", "Féminin", "France"),
    ("remy", "fr-FR-RemyMultilingualNeural", "Rémy (multilingue)", "Masculin", "France"),
];

fn voice_json(v: &(&str, &str, &str, &str, &str)) -> J {
    json!({ "id": v.1, "name": v.2, "gender": v.3, "origin": v.4 })
}

/// Apparence du cœur : le cœur holographique ou un personnage 3D (même liste que ui/character-director.js).
pub const HEART_STYLES: &[(&str, &str)] = &[
    ("classic", "Cœur holographique"),
    ("bobot", "Bobot (robot à visière)"),
    ("marvin", "Marvin (robot à grosse tête)"),
    ("petitrobot", "Petit Robot (robot rouge)"),
    ("chat", "Chat grincheux"),
];

/// Cards de type 1 (toujours actives, numéros de 1 à 99) ; toutes les autres sont de type 2 (de 100 à 199).
/// Styles de danse des personnages du cœur : « auto » (selon la musique) ou un style imposé.
pub const DANCE_STYLES: [&str; 7] = ["auto", "calme", "pop", "electro", "rock", "rap", "latino"];

pub const TYPE1_CARDS: [&str; 5] = ["bureau", "audio", "hw", "conversation", "actions"];

/// Numéros de cards choisis dans l'administration : clé de card → numéro, dans la plage de son type, sans doublon.
pub fn sanitize_card_numbers(v: &J) -> J {
    let mut out = serde_json::Map::new();
    let mut used = std::collections::HashSet::new();
    if let Some(m) = v.as_object() {
        for (k, n) in m {
            let Some(n) = n.as_i64() else { continue };
            let ok = if TYPE1_CARDS.contains(&k.as_str()) { (1..=99).contains(&n) } else { (100..=199).contains(&n) && !k.is_empty() && k.chars().count() <= 60 };
            if ok && used.insert(n) {
                out.insert(k.clone(), json!(n));
            }
        }
    }
    Value::Object(out)
}

/// Couleurs de cards reçues de l'administration : seules les quatre clés connues, en « #rrggbb » ou vides.
pub fn sanitize_card_colors(v: &J) -> J {
    let mut out = serde_json::Map::new();
    for k in ["t1n", "t1r", "t2n", "t2r"] {
        let c = js::gs(v, k);
        if c.is_empty() || crate::re!(r"^#[0-9a-fA-F]{6}$").is_match(&c) {
            out.insert(k.into(), json!(c.to_lowercase()));
        }
    }
    Value::Object(out)
}

pub fn heart_style_label(key: &str) -> Option<&'static str> {
    HEART_STYLES.iter().find(|(k, _)| *k == key).map(|(_, l)| *l)
}

/// Voix de chaque personnage : (clé, voix, nom, vitesse, hauteur, effet, phrase d'exemple).
pub const CHARACTER_VOICES: &[(&str, &str, &str, &str, &str, &str, &str)] = &[
    ("bobot", "fr-FR-RemyMultilingualNeural", "Bobot", "+5%", "+6Hz", "robot", "Bip bip ! Bobot à votre service."),
    ("marvin", "fr-FR-HenriNeural", "Marvin", "-14%", "-12Hz", "robot_deep", "Marvin. À votre service… comme toujours. Quelle joie."),
    ("petitrobot", "fr-FR-EloiseNeural", "Petit Robot", "+10%", "+10Hz", "robot_light", "Coucou ! Petit Robot est prêt, on y va !"),
    ("chat", "fr-BE-GerardNeural", "Chat grincheux", "-6%", "-5Hz", "none", "Hmpf. Le chat est là. Qu’est-ce que tu veux encore ?"),
];

pub fn character_persona(key: &str) -> Option<&'static str> {
    Some(match key {
        "bobot" => "Tu incarnes Bobot, un petit robot serviable et sympathique : ton ton est chaleureux et un brin espiègle.",
        "marvin" => "Tu incarnes Marvin, un robot dépressif et grincheux : tu réponds avec lassitude, soupirs et ironie désabusée (« Encore une tâche… quelle joie »), mais tu fais toujours ce qu’on te demande, exactement et brièvement.",
        "petitrobot" => "Tu incarnes Petit Robot, un robot enthousiaste et joyeux : tu réponds avec entrain et bonne humeur, sans en faire trop.",
        "chat" => "Tu incarnes un chat grincheux : ton ton est bougon et un peu hautain (« Hmpf »), tu soupires volontiers, mais tu rends toujours le service demandé, exactement.",
        _ => return None,
    })
}

fn fold(text: &str) -> String {
    js::fold_lower(text).trim().to_string()
}

pub fn resolve_voice(name: &str) -> Option<J> {
    if name.is_empty() {
        return Some(voice_json(&VOICES[0]));
    }
    let clean = crate::re!(r"[^a-z]").replace_all(&fold(name), "").into_owned();
    if let Some(v) = VOICES.iter().find(|v| v.0 == clean) {
        return Some(voice_json(v));
    }
    VOICES.iter().find(|v| clean.contains(v.0) || v.2.to_lowercase() == clean).map(voice_json)
}

pub const MAX_AI_CHAIN: usize = 4;

/// Valide un créneau IA `{ engine, model }` pour la chaîne d'exécution (jusqu'à 4 IA).
pub fn sanitize_ai_slot(engine: &str, model: &str) -> J {
    let eng = if engine == "agy" { "agy" } else { "claude" };
    let mdl = if model.is_empty() || model == "auto" {
        "auto".to_string()
    } else if eng == "claude" {
        if CLAUDE_MODELS.contains(&model) { model.to_string() } else { "auto".to_string() }
    } else {
        agy_model(model).unwrap_or("auto").to_string()
    };
    json!({ "engine": eng, "model": mdl })
}

/// Nettoie et borne (1 à 4 entrées) la chaîne d'IA configurée dans l'administration.
pub fn sanitize_ai_chain(v: &J, fallback_engine: &str, fallback_model: Option<&str>) -> Vec<J> {
    let mut out = Vec::new();
    if let Some(arr) = v.as_array() {
        for item in arr.iter().take(MAX_AI_CHAIN) {
            out.push(sanitize_ai_slot(&js::gs(item, "engine"), &js::gs(item, "model")));
        }
    }
    if out.is_empty() {
        out.push(sanitize_ai_slot(fallback_engine, fallback_model.unwrap_or("auto")));
    }
    out
}

#[derive(Clone)]
pub struct StatusData {
    pub engine: String,
    pub model: Option<String>,
    /// Chaîne ordonnée d'IA (1 à 4 entrées `{ engine, model }`) : l'entrée 0 est l'IA par défaut, les suivantes prennent le relais en cas d'échec.
    pub ai_chain: Vec<J>,
    pub effort: Option<String>,
    pub economy_mode: bool,
    pub voice: String,
    pub voice_name: String,
    pub voice_rate: String,
    pub listening_mode: String,
    pub theme: String,
    pub density: String,
    pub heart_style: String,
    /// Couleurs des cards du HUD : t1n / t1r (type 1, normal / réduit), t2n / t2r (type 2). Absent ou vide : défaut.
    pub card_colors: J,
    /// Numéros de cards personnalisés (clé de card → numéro) ; absent : numéro par défaut.
    pub card_numbers: J,
    /// Danse des personnages quand la musique joue : « auto » ou un style (voir DANCE_STYLES).
    pub dance_style: String,
    pub character_voice: bool,
    pub character_persona: bool,
    pub active_model: Option<String>,
    pub usage: J,
    pub weather_city: Option<String>,
    pub economy_level: String,
    /// Démarrage avec Windows (non persisté, comme en v3 : le shell détient l’état réel).
    pub autostart: bool,
}

impl StatusData {
    /// Synchronise le créneau principal (rang 0) de la chaîne avec `engine` et `model`.
    pub fn sync_primary_to_chain(&mut self) {
        let slot = sanitize_ai_slot(&self.engine, self.model.as_deref().unwrap_or("auto"));
        if self.ai_chain.is_empty() {
            self.ai_chain.push(slot);
        } else {
            self.ai_chain[0] = slot;
        }
    }
}

impl Default for StatusData {
    fn default() -> Self {
        Self {
            engine: "claude".into(),
            model: None,
            ai_chain: vec![json!({ "engine": "claude", "model": "auto" })],
            effort: None,
            economy_mode: false,
            voice: "fr-FR-DeniseNeural".into(),
            voice_name: "Denise".into(),
            voice_rate: "+0%".into(),
            listening_mode: "wakeword".into(),
            theme: "cyan".into(),
            density: "normal".into(),
            heart_style: "classic".into(),
            card_colors: json!({}),
            card_numbers: json!({}),
            dance_style: "auto".into(),
            character_voice: true,
            character_persona: true,
            active_model: None,
            usage: Value::Null,
            weather_city: None,
            economy_level: "local".into(),
            autostart: false,
        }
    }
}

pub struct Status {
    file: PathBuf,
    pub d: RefCell<StatusData>,
    pub daily: RefCell<Option<Rc<UsageDaily>>>,
}

fn opt(v: &J) -> Option<String> {
    let s = js::s(v);
    (!s.is_empty()).then_some(s)
}

impl Status {
    pub fn new(file: PathBuf) -> Self {
        let s = Self { file, d: RefCell::new(StatusData::default()), daily: RefCell::new(None) };
        s.load();
        s
    }

    fn load(&self) {
        let Some(saved) = js::read_json(&self.file) else { return };
        let mut d = self.d.borrow_mut();
        let or = |k: &str, def: &str| opt(&saved[k]).unwrap_or_else(|| def.to_string());
        d.engine = or("engine", "claude");
        d.model = opt(&saved["model"]);
        d.ai_chain = sanitize_ai_chain(&saved["aiChain"], &d.engine, d.model.as_deref());
        if saved["aiChain"].is_array() {
            d.engine = js::gs(&d.ai_chain[0], "engine");
            d.model = Some(js::gs(&d.ai_chain[0], "model"));
        }
        d.effort = opt(&saved["effort"]);
        d.economy_mode = js::truthy(&saved["economyMode"]);
        d.voice = or("voice", "fr-FR-DeniseNeural");
        d.voice_name = or("voiceName", "Denise");
        d.voice_rate = or("voiceRate", "+0%");
        d.listening_mode = or("listeningMode", "wakeword");
        d.theme = or("theme", "cyan");
        d.density = or("density", "normal");
        let hs = js::gs(&saved, "heartStyle");
        d.heart_style = if heart_style_label(&hs).is_some() { hs } else { "classic".into() };
        d.card_colors = sanitize_card_colors(&saved["cardColors"]);
        d.card_numbers = sanitize_card_numbers(&saved["cardNumbers"]);
        let ds = js::gs(&saved, "danceStyle");
        d.dance_style = if DANCE_STYLES.contains(&ds.as_str()) { ds } else { "auto".into() };
        d.character_voice = saved["characterVoice"] != json!(false);
        d.character_persona = saved["characterPersona"] != json!(false);
        d.usage = if js::truthy(&saved["usage"]) { saved["usage"].clone() } else { Value::Null };
        d.active_model = opt(&saved["activeModel"]);
        d.weather_city = opt(&saved["weatherCity"]);
        let lvl = js::gs(&saved, "economyLevel");
        d.economy_level = if ["normal", "local", "strict"].contains(&lvl.as_str()) { lvl } else { "local".into() };
    }

    pub fn save(&self) {
        let d = self.d.borrow();
        let v = json!({
            "engine": d.engine, "model": d.model, "aiChain": d.ai_chain, "effort": d.effort, "economyMode": d.economy_mode,
            "voice": d.voice, "voiceName": d.voice_name, "voiceRate": d.voice_rate, "listeningMode": d.listening_mode,
            "theme": d.theme, "density": d.density, "heartStyle": d.heart_style, "cardColors": d.card_colors, "cardNumbers": d.card_numbers, "danceStyle": d.dance_style, "characterVoice": d.character_voice,
            "characterPersona": d.character_persona, "usage": d.usage, "activeModel": d.active_model,
            "weatherCity": d.weather_city, "economyLevel": d.economy_level
        });
        let _ = js::write_json(&self.file, &v);
    }

    /// Voix réellement utilisée : celle du personnage du cœur s'il en a une (et si le réglage le
    /// permet), sinon celle des réglages.
    pub fn speech(&self) -> J {
        let d = self.d.borrow();
        if d.character_voice {
            if let Some(c) = CHARACTER_VOICES.iter().find(|c| c.0 == d.heart_style) {
                return json!({ "voice": c.1, "name": c.2, "rate": c.3, "pitch": c.4, "effect": c.5, "sample": c.6 });
            }
        }
        json!({ "voice": d.voice, "name": d.voice_name, "rate": d.voice_rate, "pitch": "+0Hz", "effect": "none" })
    }

    /// Caractère du personnage pour la consigne de l'IA (vide : aucun).
    pub fn character_line(&self) -> String {
        let d = self.d.borrow();
        if d.character_persona {
            character_persona(&d.heart_style).unwrap_or("").to_string()
        } else {
            String::new()
        }
    }

    /// Relevé du quota transmis par Claude Code pendant une tâche.
    pub fn apply_rate_limit(&self, info: &J) -> bool {
        if !js::truthy(info) {
            return false;
        }
        let windows = &info["unifiedWindows"];
        let five = &windows["five_hour"];
        let week = &windows["seven_day"];
        let pct = |w: &J| (js::num(&w["utilization"]).unwrap_or(0.0) * 100.0).round() as i64;
        let five_reset = if js::truthy(&five["resetsAt"]) { five["resetsAt"].clone() } else if js::truthy(&info["resetsAt"]) { info["resetsAt"].clone() } else { Value::Null };
        let week_reset = if js::truthy(&week["resetsAt"]) { week["resetsAt"].clone() } else { Value::Null };
        self.d.borrow_mut().usage = json!({
            "five": { "pct": pct(five), "resetsAt": five_reset },
            "week": { "pct": pct(week), "resetsAt": week_reset },
            "overage": js::truthy(&info["isUsingOverage"]),
            "at": js::now_ms()
        });
        self.save();
        true
    }

    pub fn snapshot(&self) -> J {
        let speech = self.speech();
        let d = self.d.borrow();
        json!({
            "type": "status",
            "engine": d.engine,
            "model": d.active_model.clone().or_else(|| d.model.clone()),
            "requested": d.model,
            "aiChain": d.ai_chain,
            "effort": d.effort,
            "economyMode": d.economy_mode,
            "voice": d.voice,
            "voiceName": d.voice_name,
            "voiceRate": d.voice_rate,
            "listeningMode": d.listening_mode,
            "theme": d.theme,
            "density": d.density,
            "heartStyle": d.heart_style,
            "cardColors": d.card_colors,
            "cardNumbers": d.card_numbers,
            "danceStyle": d.dance_style,
            "characterVoice": d.character_voice,
            "characterPersona": d.character_persona,
            "speech": speech,
            "usage": d.usage,
            "economyLevel": d.economy_level,
            "today": self.daily.borrow().as_ref().map(|u| u.snapshot()).unwrap_or(Value::Null)
        })
    }

    /// Commandes de réglage dites ou tapées.
    pub fn parse(&self, text: &str) -> Option<J> {
        let raw = fold(text).replace(['?', '!'], "");
        let raw = crate::re!(r"\.+$").replace(&raw, "").to_string();
        let r = raw.as_str();

        // 0. Vibe coding
        if let Some(m) = crate::re!(r"(?i)^/vibe(?:\s+(court|ultra[\s-]court|standard|detaille|détaillé|off|on))?").captures(r) {
            return Some(json!({ "kind": "vibe", "mode": m.get(1).map(|x| x.as_str()).unwrap_or("statut") }));
        }

        // 1. Moteur
        let slash_engine = crate::re!(r"^/(?:engine|moteur|backend)\s+(\S+)").captures(r).map(|m| m[1].to_string());
        let spoken_engine = crate::fre!(r"\b(?:passe (?:en|sur)|utilise|bascule (?:en|sur)|moteur|sur)\s+(claude\s+code|claude|agy|agi|hgy|a,?\s*g,?\s*y|ajy|aji|antigravity|gemini)(?!\s*[-0-9.a-z])")
            .captures(r)
            .ok()
            .flatten()
            .and_then(|m| m.get(1).map(|x| x.as_str().to_string()));
        if let Some(eng) = slash_engine.or(spoken_engine) {
            let e = crate::re!(r"\s+").replace_all(&eng.replace(',', ""), " ").trim().to_string();
            if let Some(v) = engine_of(&e) {
                return Some(json!({ "kind": "engine", "value": v }));
            }
        }

        // 2. Modèle
        let slash_model = crate::re!(r"^/model\s+(\S+)").captures(r).map(|m| m[1].to_string());
        let spoken_model = crate::re!(r"\b(?:passe (?:en|sur)|utilise|bascule (?:en|sur)|modele)\s+([a-z0-9.-]+(?:\s+[a-z0-9.-]+)?)\b").captures(r).map(|m| m[1].to_string());
        if let Some(cand) = slash_model.clone().or(spoken_model) {
            let clean = crate::re!(r"\s+").replace_all(cand.trim(), "-").into_owned();
            if let Some(direct) = model_of(&clean).or_else(|| model_of(cand.trim())) {
                let engine = if is_agy_model_key(&direct) || clean.starts_with("gemini") { "agy" } else { "claude" };
                return Some(json!({ "kind": "model", "value": direct, "engine": engine }));
            }
            if let Some(sm) = slash_model {
                let engine = if sm.starts_with("gemini") { json!("agy") } else { Value::Null };
                return Some(json!({ "kind": "model", "value": sm, "engine": engine }));
            }
        }

        // 3. Effort
        if let Some(m) = crate::re!(r"^/effort\s+(\S+)").captures(r) {
            if EFFORT_VALUES.contains(&&m[1]) {
                return Some(json!({ "kind": "effort", "value": &m[1] }));
            }
        }
        if let Some(m) = crate::re!(r"\beffort\s+(?:au\s+|a\s+|en\s+)?([a-z ]+)").captures(r) {
            let label = m[1].trim();
            if let Some(v) = effort_of(label).or_else(|| effort_of(label.split(' ').next().unwrap_or(""))) {
                return Some(json!({ "kind": "effort", "value": v }));
            }
        }

        // 4. Voix
        if let Some(m) = crate::re!(r"^/(?:voix|voice)(?:\s+(\S+))?$").captures(r) {
            match m.get(1) {
                None => return Some(json!({ "kind": "list_voices" })),
                Some(n) => {
                    if let Some(v) = resolve_voice(n.as_str()) {
                        return Some(json!({ "kind": "voice", "value": v }));
                    }
                }
            }
        }
        if crate::re!(r"\b(?:quelles?\s+(?:sont\s+les\s+)?voix|liste\s+(?:les\s+)?voix|quelles?\s+voix\s+as-tu)\b").is_match(r) {
            return Some(json!({ "kind": "list_voices" }));
        }
        if let Some(m) = crate::re!(r"\b(?:prends?|utilise|mets?|choisis?|change\s+de\s+voix\s+pour|voix)\s+(?:la\s+voix\s+d['e]?\s*|la\s+voix\s+de\s*)?([a-z]+)\b").captures(r) {
            if let Some(v) = resolve_voice(&m[1]) {
                return Some(json!({ "kind": "voice", "value": v }));
            }
        }

        // 5. Vitesse
        if let Some(m) = crate::re!(r"^/(?:rate|vitesse)\s+([+\-0-9%a-z]+)").captures(r) {
            let mut v = m[1].to_string();
            if v == "normale" || v == "normal" {
                v = "+0%".into();
            } else if v == "rapide" {
                v = "+20%".into();
            } else if v == "lente" {
                v = "-20%".into();
            } else if !v.ends_with('%') {
                v = format!("{}%", if v.starts_with('+') || v.starts_with('-') { v.clone() } else { format!("+{v}") });
            }
            return Some(json!({ "kind": "rate", "value": v }));
        }
        if crate::re!(r"\b(?:parle\s+)?(?:tres|beaucoup\s+plus)\s+vite\b").is_match(r) {
            return Some(json!({ "kind": "rate", "value": "+30%" }));
        }
        if crate::re!(r"\b(?:parle\s+)?(?:un\s+peu\s+)?plus\s+vite\b|\baccelere\b").is_match(r) {
            return Some(json!({ "kind": "rate", "value": "+15%" }));
        }
        if crate::re!(r"\b(?:parle\s+)?(?:tres|beaucoup\s+plus)\s+(?:lentement|moins\s+vite)\b").is_match(r) {
            return Some(json!({ "kind": "rate", "value": "-25%" }));
        }
        if crate::re!(r"\b(?:parle\s+)?(?:un\s+peu\s+)?(?:moins\s+vite|plus\s+lentement)\b|\bralentis\b").is_match(r) {
            return Some(json!({ "kind": "rate", "value": "-15%" }));
        }
        if crate::re!(r"\b(?:vitesse\s+normale|parle\s+normalement|reinitialise\s+la\s+vitesse)\b").is_match(r) {
            return Some(json!({ "kind": "rate", "value": "+0%" }));
        }

        // 6. Quota
        let asks_usage = crate::re!(r"^/?(usage|utilisation|quota|consommation)$").is_match(r)
            || crate::re!(r"\b(quel|quelle|combien|ou en est|où en est).{0,20}(utilisation|quota|consommation)\b").is_match(r)
            || crate::re!(r"\b(mon|ton|notre)\s+(utilisation|quota|consommation)\b").is_match(r)
            || crate::re!(r"\bil (me |te )?reste combien\b").is_match(r)
            || crate::re!(r"^/?(tokens?|jetons?)$").is_match(r)
            || crate::re!(r"\bcombien (de |d['’ ]?)?(tokens?|jetons?)\b").is_match(r)
            || crate::re!(r"\bcombien d['’ ]?appels? (aux|a l['’ ]?|a une) ?ia\b").is_match(r);
        if asks_usage {
            return Some(json!({ "kind": "usage" }));
        }

        // 7. Zones
        if crate::re!(r"^/zones?$").is_match(r) || crate::re!(r"\b(?:quelles?\s+(?:sont\s+les\s+)?zones|liste\s+(?:les\s+)?zones)\b").is_match(r) {
            return Some(json!({ "kind": "list_zones" }));
        }
        let trimmed = text.trim();
        if let Some(m) = crate::re!(r"(?i)^/zone\s+add\s+([^\s]+)\s+([^\s]+)(?:\s+([^\s]+))?(?:\s+([^\s]+))?$").captures(trimmed) {
            return Some(json!({
                "kind": "zone_add", "name": &m[1], "path": &m[2],
                "read": m.get(3).map(|x| x.as_str()).unwrap_or("free"), "write": m.get(4).map(|x| x.as_str()).unwrap_or("code")
            }));
        }
        if let Some(m) = crate::re!(r"(?i)^/zone\s+(?:remove|delete|rm)\s+([^\s]+)$").captures(trimmed) {
            return Some(json!({ "kind": "zone_remove", "name": &m[1] }));
        }

        // 8. Mode d'écoute
        if let Some(m) = crate::re!(r"^/(?:mode|ecoute|listening)(?:\s+(\S+))?$").captures(r) {
            match m.get(1) {
                None => return Some(json!({ "kind": "get_listening_mode" })),
                Some(x) => {
                    let m = x.as_str().to_lowercase();
                    if ["wakeword", "mot", "motdeveil", "eveil"].contains(&m.as_str()) {
                        return Some(json!({ "kind": "listening_mode", "value": "wakeword" }));
                    }
                    if ["ptt", "push", "push-to-talk", "appui", "reunion"].contains(&m.as_str()) {
                        return Some(json!({ "kind": "listening_mode", "value": "ptt" }));
                    }
                    if ["free", "libre", "continu", "continue"].contains(&m.as_str()) {
                        return Some(json!({ "kind": "listening_mode", "value": "free" }));
                    }
                }
            }
        }
        let spaced = crate::re!(r"\s+").replace_all(&r.replace(['-', ','], " "), " ").trim().to_string();
        if crate::re!(r"^(?:(?:relance|redemarre|recharge) (?:toi|jarvis)|redemarre|(?:mets|met) toi a jour|applique (?:la|les) mises? a jour)(?: (?:completement|entierement|maintenant|s il te plait))*$").is_match(&spaced) {
            return Some(json!({ "kind": "restart_jarvis" }));
        }
        if crate::re!(r"^(?:(?:ok|bon|allez|alors)[\s,]+)?(?:ecoutez?|ecoutes)[\s-]+moi(?:[\s,]+(?:bien|maintenant|en\s+permanence|en\s+continu|tout\s+le\s+temps))?(?:[\s,]+s['’ ]?il\s+te\s+plait)?\s*$").is_match(r) {
            return Some(json!({ "kind": "listening_mode", "value": "free" }));
        }
        if crate::re!(r"^(?:(?:ok|bon|allez|merci|c['’ ]?est\s+bon)[\s,]+)?(?:tu\s+peux\s+)?(?:arretes?|arreter|cesses?|cesser|stoppe)[\s,]+(?:de\s+m['’ ]?ecouter|d['’ ]?ecouter|l['’ ]?ecoute(?:\s+(?:permanente|continue|libre))?)(?:[\s,]+(?:maintenant|s['’ ]?il\s+te\s+plait))*\s*$").is_match(r) {
            return Some(json!({ "kind": "listening_mode", "value": "wakeword" }));
        }
        if crate::re!(r"\b(?:mode\s+)?(?:mot\s+d[' ]?eveil|active\s+le\s+mot\s+d[' ]?eveil)\b").is_match(r) {
            return Some(json!({ "kind": "listening_mode", "value": "wakeword" }));
        }
        if crate::re!(r"\b(?:mode\s+)?(?:appui\s+pour\s+parler|push\s+to\s+talk|mode\s+reunion)\b").is_match(r) {
            return Some(json!({ "kind": "listening_mode", "value": "ptt" }));
        }
        if crate::re!(r"\b(?:ecoute\s+libre|mode\s+libre|ecoute\s+continue|desactive\s+le\s+mot\s+d[' ]?eveil)\b").is_match(r) {
            return Some(json!({ "kind": "listening_mode", "value": "free" }));
        }
        if crate::re!(r"\b(?:quel\s+est\s+le\s+mode\s+d[' ]?ecoute|mode\s+d[' ]?ecoute)\b").is_match(r) {
            return Some(json!({ "kind": "get_listening_mode" }));
        }

        // 9. Autorisation temporaire d'écriture
        if let Some(m) = crate::re!(r"(?i)^/(?:autoriser|auth)\s+([a-zA-Z0-9_\-\\ ]+?)(?:\s+(\d+h?|\d+m(?:in)?))?$").captures(r) {
            let dur = m.get(2).map(|x| x.as_str()).unwrap_or("60");
            let digits: i64 = crate::re!(r"^\d+").find(dur).and_then(|d| d.as_str().parse().ok()).unwrap_or(0);
            let minutes = if dur.ends_with('h') {
                digits * 60
            } else if dur.ends_with('m') || dur.ends_with("min") {
                digits
            } else if digits != 0 {
                digits
            } else {
                60
            };
            return Some(json!({ "kind": "authorize", "zone": m[1].trim(), "duration": minutes }));
        }

        // 10. Journal et dernière réponse
        if crate::re!(r"(?i)^/(?:journal|historique|history)$").is_match(r) || crate::re!(r"(?i)\b(?:affiche|montre|ouvre|masque|ferme)\s+(?:le\s+)?(?:journal|historique)\b").is_match(r) {
            return Some(json!({ "kind": "toggle_journal" }));
        }
        if crate::re!(r"(?i)^/(?:verifier|verify|integrite|intégrité)$").is_match(r)
            || crate::re!(r"(?i)\b(?:verifie|vérifie|teste)\s+(?:l['’]integrite|l['’]intégrité|le\s+journal|le\s+scellement)\b").is_match(r)
        {
            return Some(json!({ "kind": "verify_journal" }));
        }
        if crate::re!(r"(?i)^/(?:reponse|réponse|derniere|dernière|full)$").is_match(r)
            || crate::re!(r"(?i)\b(?:relis|affiche|montre|donne)\s+(?:la\s+)?(?:derniere\s+|dernière\s+)?(?:reponse|réponse)(?:\s+complete|\s+complète)?\b").is_match(r)
        {
            return Some(json!({ "kind": "show_last_answer" }));
        }

        // 11. Démarrage avec Windows
        if let Some(m) = crate::re!(r"(?i)^/autostart(?:\s+(on|off|true|false|oui|non|active|desactive))?$").captures(r) {
            let val = m.get(1).map(|x| ["on", "true", "oui", "active"].contains(&x.as_str().to_lowercase().as_str())).unwrap_or(true);
            return Some(json!({ "kind": "autostart", "value": val }));
        }
        if crate::re!(r"\b(?:active|active\s+le|active\s+le\s+mode)\s+(?:demarrage|démarrage)\s+(?:avec\s+windows|automatique)\b").is_match(r) {
            return Some(json!({ "kind": "autostart", "value": true }));
        }
        if crate::re!(r"\b(?:desactive|désactive|arrete|arrête)\s+(?:le\s+)?(?:demarrage|démarrage)\s+(?:avec\s+windows|automatique)\b").is_match(r) {
            return Some(json!({ "kind": "autostart", "value": false }));
        }

        // 11 bis. Niveau d'économie de tokens
        let level = crate::re!(r"^/(?:economie|économie|eco)\s+(strict|stricte|normal|normale|local|locale)$")
            .captures(r)
            .or_else(|| crate::re!(r"\b(?:economie|économie|niveau d['’ ]?economie)\s+(?:de tokens?\s+)?(strict|stricte|normal|normale|local d['’ ]?abord|locale?)\b").captures(r))
            .or_else(|| crate::re!(r"\bmode\s+(strict|local d['’ ]?abord)\b").captures(r));
        if let Some(m) = level {
            let v = if m[1].starts_with("strict") { "strict" } else if m[1].starts_with("normal") { "normal" } else { "local" };
            return Some(json!({ "kind": "economy_level", "value": v }));
        }

        // 12. Économie de quota
        if let Some(m) = crate::re!(r"(?i)^/(?:economie|économie|eco|quota_eco)(?:\s+(on|off|auto|true|false|active|desactive))?$").captures(r) {
            let val = match m.get(1) {
                Some(x) => ["on", "auto", "true", "active"].contains(&x.as_str().to_lowercase().as_str()),
                None => !self.d.borrow().economy_mode,
            };
            return Some(json!({ "kind": "economy", "value": val }));
        }
        if crate::re!(r"\b(?:active|mets|active\s+le|passe\s+en)\s+(?:mode\s+)?(?:economie|économie|eco)\b").is_match(r) {
            return Some(json!({ "kind": "economy", "value": true }));
        }
        if crate::re!(r"\b(?:desactive|coupe|arrete|arrête|quitte)\s+(?:le\s+)?(?:mode\s+)?(?:economie|économie|eco)\b").is_match(r) {
            return Some(json!({ "kind": "economy", "value": false }));
        }

        // 13. Sourdine contextuelle
        if let Some(m) = crate::re!(r"(?i)^/(?:sourdine|mute|discret|silence)(?:\s+(auto|on|off|active|desactive))?$").captures(r) {
            let val = match m.get(1).map(|x| x.as_str().to_lowercase()) {
                Some(x) if ["on", "active", "1"].contains(&x.as_str()) => "on",
                Some(x) if ["off", "desactive", "0"].contains(&x.as_str()) => "off",
                _ => "auto",
            };
            return Some(json!({ "kind": "context_mute", "value": val }));
        }
        if crate::re!(r"\b(?:mode\s+)?(?:discret|silencieux|passe\s+en\s+silence)\b").is_match(r) {
            return Some(json!({ "kind": "context_mute", "value": "on" }));
        }
        if crate::re!(r"\b(?:sourdine\s+automatique|sourdine\s+auto)\b").is_match(r) {
            return Some(json!({ "kind": "context_mute", "value": "auto" }));
        }
        if crate::re!(r"\b(?:desactive|coupe|arrete|arrête)\s+(?:la\s+)?(?:sourdine|le\s+mode\s+discret|le\s+mode\s+silencieux)\b").is_match(r)
            || crate::re!(r"\b(?:remets|reactive|réactive)\s+(?:la\s+)?voix\b").is_match(r)
        {
            return Some(json!({ "kind": "context_mute", "value": "off" }));
        }
        if crate::re!(r"\b(?:active|mets)\s+(?:la\s+)?(?:sourdine|le\s+silence)\b").is_match(r) {
            return Some(json!({ "kind": "context_mute", "value": "on" }));
        }

        // 13 bis. Apparence du cœur
        if let Some(m) = crate::re!(r"^/(?:coeur|cœur|personnage|apparence)(?:\s+(\S+))?$").captures(r) {
            let arg = m.get(1).map(|x| x.as_str()).unwrap_or("");
            if crate::re!(r"^(?:classique|classic|coeur|cœur|orbe|holographique|normal|standard)$").is_match(arg) {
                return Some(json!({ "kind": "heart_style", "value": "classic" }));
            }
            if heart_style_label(arg).is_some() {
                return Some(json!({ "kind": "heart_style", "value": arg }));
            }
            let alias = match arg {
                "robot" => Some("bobot"),
                "personnage" | "3d" => Some("marvin"),
                "petit" | "rouge" => Some("petitrobot"),
                "grincheux" | "chaton" => Some("chat"),
                _ => None,
            };
            return Some(match alias {
                Some(a) => json!({ "kind": "heart_style", "value": a }),
                None => json!({ "kind": "list_heart_styles" }),
            });
        }
        const CHARACTER: &str = r"(?:le\s+|la\s+)?(?:personnage|robot\s+marvin|marvin|bobot|petit\s+robot|robot\s+rouge|chat(?:\s+grincheux)?|robot)(?:\s+(?:au\s+centre|dans\s+le\s+c(?:oe|œ)ur|a\s+la\s+place\s+du\s+c(?:oe|œ)ur))?(?:[\s,]+(?:s['’ ]?il\s+(?:te|vous)\s+plait|stp|svp|merci))?\s*$";
        static CLASSIC: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(&format!(r"\b(?:(?:coeur|cœur)\s+(?:classique|holographique|d['’ ]?origine|normal)|(?:remets|reprends|retour\s+au|reviens\s+au)\s+(?:le\s+)?(?:coeur|cœur)(?:\s+classique)?|(?:enleve|retire|cache|masque|desactive|supprime|vire)\s+{CHARACTER})")).unwrap()
        });
        if CLASSIC.is_match(r) {
            return Some(json!({ "kind": "heart_style", "value": "classic" }));
        }
        static SHOWN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(&format!(r"\b(?:(?:affiche|montre|mets|active|lance|prends|reactive)\s+|mode\s+|passe\s+(?:en|au)\s+(?:mode\s+)?){CHARACTER}")).unwrap()
        });
        if let Some(m) = SHOWN.find(r) {
            let who = m.as_str();
            let value = if crate::re!(r"petit\s+robot|robot\s+rouge").is_match(who) {
                "petitrobot"
            } else if who.contains("chat") {
                "chat"
            } else if who.contains("bobot") || (crate::re!(r"\brobot\b").is_match(who) && !who.contains("marvin")) {
                "bobot"
            } else {
                "marvin"
            };
            return Some(json!({ "kind": "heart_style", "value": value }));
        }

        // 14. Thèmes
        if let Some(m) = crate::re!(r"(?i)^/(?:theme|thème|style)(?:\s+(\S+))?$").captures(r) {
            let Some(t) = m.get(1) else { return Some(json!({ "kind": "list_themes" })) };
            let t = t.as_str().to_lowercase();
            let theme = if ["cyan", "gold", "matrix", "crimson", "violet"].contains(&t.as_str()) {
                t.clone()
            } else {
                match t.as_str() {
                    "bleu" | "hologramme" => "cyan",
                    "or" | "dore" | "doré" | "ambre" => "gold",
                    "vert" | "emeraude" | "émeraude" => "matrix",
                    "rouge" | "rubis" => "crimson",
                    "pourpre" | "mauve" => "violet",
                    _ => "cyan",
                }
                .to_string()
            };
            return Some(json!({ "kind": "theme", "value": theme }));
        }
        if crate::re!(r"\b(?:theme|thème)\s+(dore|doré|or|ambre)\b|\b(?:passe\s+en\s+mode\s+)?(?:dore|doré|stark)\b").is_match(r) {
            return Some(json!({ "kind": "theme", "value": "gold" }));
        }
        if crate::re!(r"\b(?:theme|thème)\s+(vert|matrix|emeraude|émeraude)\b").is_match(r) {
            return Some(json!({ "kind": "theme", "value": "matrix" }));
        }
        if crate::re!(r"\b(?:theme|thème)\s+(rouge|crimson|rubis)\b").is_match(r) {
            return Some(json!({ "kind": "theme", "value": "crimson" }));
        }
        if crate::re!(r"\b(?:theme|thème)\s+(violet|synthwave|amethyste|améthyste|pourpre)\b").is_match(r) {
            return Some(json!({ "kind": "theme", "value": "violet" }));
        }
        if crate::re!(r"\b(?:theme|thème)\s+(?:cyan|bleu|d['’ ]?origine|standard|hologramme)\b").is_match(r) {
            return Some(json!({ "kind": "theme", "value": "cyan" }));
        }

        // 15. Densité
        if let Some(m) = crate::re!(r"(?i)^/(?:densite|densité|density)(?:\s+(\S+))?$").captures(r) {
            let d = m.get(1).map(|x| x.as_str().to_lowercase()).unwrap_or_else(|| "normal".into());
            let val = if d == "compact" || d == "large" { d } else { "normal".into() };
            return Some(json!({ "kind": "density", "value": val }));
        }
        if crate::re!(r"\b(?:mode\s+compact|affichage\s+compact|densite\s+compacte)\b").is_match(r) {
            return Some(json!({ "kind": "density", "value": "compact" }));
        }
        if crate::re!(r"\b(?:mode\s+large|affichage\s+grand|grand\s+affichage|densite\s+large)\b").is_match(r) {
            return Some(json!({ "kind": "density", "value": "large" }));
        }
        if crate::re!(r"\b(?:mode\s+normal|affichage\s+normal|densite\s+normale)\b").is_match(r) {
            return Some(json!({ "kind": "density", "value": "normal" }));
        }

        // 16. Empreinte vocale
        let cap_name = |s: &str| {
            let low = s.to_lowercase();
            js::capitalize(&low)
        };
        if let Some(m) = crate::re!(r"(?i)^/(?:locuteur|speaker|voixprint)(?:\s+(\S+)(?:\s+(\S+))?)?$").captures(r) {
            let sub = m.get(1).map(|x| x.as_str().to_lowercase()).unwrap_or_default();
            let arg = m.get(2).map(|x| x.as_str().to_string());
            if ["apprendre", "enrol", "enroll", "learn", "enregistrer", "ajouter", "add"].contains(&sub.as_str()) {
                return Some(json!({ "kind": "speaker_enroll", "name": arg.map(|a| cap_name(&a)) }));
            }
            if ["supprimer", "delete", "remove", "rm", "retirer"].contains(&sub.as_str()) {
                return Some(json!({ "kind": "speaker_delete", "target": arg.map(|a| cap_name(&a)) }));
            }
            if sub == "liste" || sub == "list" {
                return Some(json!({ "kind": "speaker_list" }));
            }
            if ["on", "activer", "active", "true", "1"].contains(&sub.as_str()) {
                return Some(json!({ "kind": "speaker_mode", "enabled": true }));
            }
            if ["off", "desactiver", "desactive", "false", "0"].contains(&sub.as_str()) {
                return Some(json!({ "kind": "speaker_mode", "enabled": false }));
            }
            if sub == "seuil" || sub == "threshold" {
                let v: f64 = arg.as_deref().and_then(|a| crate::re!(r"^[+-]?\d*\.?\d+").find(a).and_then(|x| x.as_str().parse().ok())).unwrap_or(0.65);
                return Some(json!({ "kind": "speaker_threshold", "value": v }));
            }
            if ["reset", "effacer", "reinitialiser"].contains(&sub.as_str()) {
                return Some(json!({ "kind": "speaker_reset" }));
            }
            return Some(json!({ "kind": "speaker_status" }));
        }
        if let Some(m) = crate::re!(r"(?i)\b(?:enregistre|apprends|m[eé]morise|ajoute)\s+(?:la\s+voix\s+de\s+|l\s+empreinte\s+de\s+)([a-zA-ZÀ-ÖØ-öø-ÿ'-]+)").captures(r) {
            return Some(json!({ "kind": "speaker_enroll", "name": cap_name(m[1].trim()) }));
        }
        if let Some(m) = crate::re!(r"(?i)\b(?:supprime|efface|retire|oublie)\s+(?:la\s+voix\s+de\s+|l\s+empreinte\s+de\s+)([a-zA-ZÀ-ÖØ-öø-ÿ'-]+)").captures(r) {
            return Some(json!({ "kind": "speaker_delete", "target": cap_name(m[1].trim()) }));
        }
        if crate::re!(r"(?i)\b(?:liste\s+(?:les\s+)?locuteurs|qui\s+(?:a\s+son\s+empreinte|est\s+enregistr[eé])|quelles\s+sont\s+les\s+empreintes|quelles\s+voix\s+sont\s+enregistr[eé]es)\b").is_match(r) {
            return Some(json!({ "kind": "speaker_list" }));
        }
        if crate::re!(r"\b(?:enregistre|apprends|m[eé]morise)\s+ma\s+voix\b").is_match(r) {
            return Some(json!({ "kind": "speaker_enroll" }));
        }
        if crate::re!(r"\b(?:ne\s+r[eé]ponds?\s+qu['’][aà]\s+ma\s+voix|active\s+(?:la\s+)?reconnaissance\s+(?:du\s+)?locuteur)\b").is_match(r) {
            return Some(json!({ "kind": "speaker_mode", "enabled": true }));
        }
        if crate::re!(r"\b(?:d[eé]sactive\s+(?:la\s+)?reconnaissance\s+(?:du\s+)?locuteur|accepte\s+toutes\s+les\s+voix)\b").is_match(r) {
            return Some(json!({ "kind": "speaker_mode", "enabled": false }));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaine_ia_nettoyee_et_bornee_a_quatre() {
        let raw = json!([
            { "engine": "agy", "model": "gemini-3.8-flash-high" },
            { "engine": "claude", "model": "sonnet" },
            { "engine": "agy", "model": "inconnu" },
            { "engine": "claude", "model": "opus" },
            { "engine": "claude", "model": "haiku" }
        ]);
        let chain = sanitize_ai_chain(&raw, "claude", Some("auto"));
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0], json!({ "engine": "agy", "model": "gemini-3.8-flash-high" }));
        assert_eq!(chain[1], json!({ "engine": "claude", "model": "sonnet" }));
        assert_eq!(chain[2], json!({ "engine": "agy", "model": "auto" }));
        assert_eq!(chain[3], json!({ "engine": "claude", "model": "opus" }));

        let empty = sanitize_ai_chain(&json!([]), "claude", Some("haiku"));
        assert_eq!(empty, vec![json!({ "engine": "claude", "model": "haiku" })]);
    }

    #[test]
    fn chaine_ia_effort_agy_aligne_sur_modele() {
        use crate::orch::Orchestrator;
        assert_eq!(Orchestrator::effort_for_model("agy", "gemini-3.8-flash-high", Some("medium".into())).as_deref(), Some("high"));
        assert_eq!(Orchestrator::effort_for_model("agy", "gemini-3.8-flash-medium", Some("high".into())).as_deref(), Some("medium"));
        assert_eq!(Orchestrator::effort_for_model("agy", "gemini-3.8-flash-low", Some("medium".into())).as_deref(), Some("low"));
        assert_eq!(Orchestrator::effort_for_model("claude", "haiku", Some("medium".into())).as_deref(), Some("medium"));
    }
}
