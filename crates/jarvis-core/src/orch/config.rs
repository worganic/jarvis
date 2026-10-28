// Réglages persistants de Jarvis (orchestrator/config.js) : profil de dialogue, voix, widget,
// Bureau JARVIS, tmux, projets, optimisation… Éditables depuis la page d'administration.
// Le fichier vit dans data/ : Claude Code et AGY n'y ont jamais accès (chemins gardés).

use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::js::{self, J};

pub fn tone(key: &str) -> Option<&'static str> {
    Some(match key {
        "complice" => "Ton chaleureux et complice, comme un assistant proche.",
        "majordome" => "Ton de majordome britannique : courtois, posé, pince-sans-rire à la manière du Jarvis d'Iron Man.",
        "professionnel" => "Ton professionnel, neutre et précis.",
        "direct" => "Ton direct et factuel : va droit au but, sans formule de politesse superflue.",
        "enjoue" => "Ton enjoué et dynamique, avec une pointe d’humour.",
        _ => return None,
    })
}

pub const TONE_KEYS: &[&str] = &["complice", "majordome", "professionnel", "direct", "enjoue"];

pub fn verbosity(key: &str) -> Option<&'static str> {
    Some(match key {
        "tres-court" => "Réponds en une seule phrase courte.",
        "court" => "Réponds en une à trois phrases courtes.",
        "detaille" => "Réponds de façon complète mais toujours parlée, en cinq phrases au maximum.",
        _ => return None,
    })
}

pub const VERBOSITY_KEYS: &[&str] = &["tres-court", "court", "detaille"];

pub fn defaults() -> J {
    json!({
        "profile": { "name": "", "address": "tu", "honorific": "", "tone": "complice", "verbosity": "court", "humor": 1, "instructions": "" },
        "voice": {
            "backend": "livekit",
            "livekit": {
                "port": 7880, "capture": "native", "turnDetector": "v1-mini",
                "minEndpointingDelay": 0.6, "maxEndpointingDelay": 2.5, "vadSilence": 0.3,
                "allowInterruptions": true, "minInterruptionDuration": 0.3, "resumeFalseInterruption": true,
                "echoCancellation": true, "noiseSuppression": true, "autoGainControl": true,
                "systemAudioReference": true, "sttModel": "turbo", "sttBeam": 2, "debugAudio": false, "sttRawAudio": true,
                "wakeEngine": "phonetique", "wakeModel": "jarvis.onnx", "wakeThreshold": 0.5
            }
        },
        "widget": { "size": 80, "opacity": 0.9, "clickThrough": true, "ghost": false },
        "desk": { "enabled": false, "desktop": 2, "monitor": 0, "otherScreens": true, "wallpaper": false, "wallpaperDim": 0.35 },
        "audio": { "jarvisVolume": 100, "musicSilence": true },
        "tmux": {
            "launcher": "native", "distro": "Ubuntu", "tmuxPath": "psmux", "session": "jarvis",
            "claudeCommand": "claude", "defaultDir": "", "enterDelayMs": 250, "showTerminal": true
        },
        "projects": { "root": "E:\\www\\A-En cours" },
        "briefing": { "auto": true },
        "memoryAuto": { "enabled": false, "idleMinutes": 5 },
        "announcements": { "quietHours": true, "quietStart": "22:30", "quietEnd": "07:30", "minGapSeconds": 30 },
        "presence": { "enabled": false, "awayMinutes": 5, "privacy": true, "pauseMusic": false },
        "visio": { "meetSilence": true, "pauseListening": true },
        "shopping": { "maxOffers": 10, "stores": "" },
        "mailWatch": { "enabled": true, "minutes": 5 },
        "performance": {
            "voiceListening": true, "semanticMemory": true, "hardwareMonitoring": true, "autoEco": true, "audioDeviceWatch": true,
            "metricsLog": true, "orbDetail": true, "voiceWaveform": true, "cards": true, "hudBackground": true, "maxFps": 30, "autoMode": false
        },
        "vibeCoding": { "enabled": true, "verbosite": "ultra-court", "cropSize": 500, "moteurParDefaut": "agy" },
        "aiWatch": { "enabled": true, "notifyDone": true, "notifyWaiting": true, "speak": true },
        "backup": { "enabled": true, "dir": "" },
        // Mises à jour d'un Jarvis installé (update.rs) : « auto », « demander » ou « manuel ».
        "update": { "mode": "demander" },
        // Alerte quand les compilations (target/) dépassent alertGb gigaoctets.
        "buildSpace": { "alert": true, "alertGb": 40 },
        "agy": { "mode": "bypass" }
    })
}

pub const MAX_FPS_CHOICES: &[i64] = &[0, 20, 30, 60];

/// Préréglages d'optimisation : un clic règle toutes les options.
pub fn performance_presets() -> J {
    let perf = |vl, sm, hm, ae, adw, ml, od, vw, cards, hb, fps, auto| {
        json!({
            "voiceListening": vl, "semanticMemory": sm, "hardwareMonitoring": hm, "autoEco": ae, "audioDeviceWatch": adw,
            "metricsLog": ml, "orbDetail": od, "voiceWaveform": vw, "cards": cards, "hudBackground": hb, "maxFps": fps, "autoMode": auto
        })
    };
    json!({
        "complet": {
            "label": "Complet", "description": "Tout est actif : rendu maximal, écoute orale, mémoire sémantique.",
            "performance": perf(true, true, true, true, true, true, true, true, true, true, 0, false),
            "voice": { "livekit": { "sttBeam": 2, "systemAudioReference": true } }
        },
        "auto": {
            "label": "Automatique", "description": "Comme « Complet », mais Jarvis réduit lui-même l’affichage quand le PC est chargé et le rétablit ensuite.",
            "performance": perf(true, true, true, true, true, true, true, true, true, true, 0, true),
            "voice": { "livekit": { "sttBeam": 2, "systemAudioReference": true } }
        },
        "equilibre": {
            "label": "Équilibré", "description": "Tout reste utilisable ; l’affichage est plafonné à 60 images/s et le journal de mesure coupé.",
            "performance": perf(true, true, true, true, true, false, true, true, true, true, 60, false),
            "voice": { "livekit": { "sttBeam": 2, "systemAudioReference": true } }
        },
        "econome": {
            "label": "Économe", "description": "La voix reste active ; mémoire sémantique, fond du HUD et cœur détaillé coupés, 30 images/s.",
            "performance": perf(true, false, true, true, false, false, false, true, true, false, 30, false),
            "voice": { "livekit": { "sttBeam": 1, "systemAudioReference": false } }
        },
        "minimal": {
            "label": "Minimal (texte)", "description": "Mode texte : plus d’écoute ni de parole, plus de surveillance, affichage réduit à l’essentiel.",
            "performance": perf(false, false, false, false, false, false, false, false, false, false, 30, false),
            "voice": { "livekit": { "sttBeam": 1, "systemAudioReference": false } }
        }
    })
}

/// Nombre JSON au format JS (entier si la valeur est entière).
pub fn jnum(f: f64) -> Value {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 {
        json!(f as i64)
    } else if f.is_finite() {
        json!(f)
    } else {
        Value::Null
    }
}

// Fusion profonde bornée aux clés connues : un fichier corrompu ou une clé inventée depuis
// l'interface ne peut pas ajouter de réglage que le code ne connaît pas.
pub fn merge_known(base: &J, patch: &J) -> J {
    let Value::Object(b) = base else { return base.clone() };
    let mut out = b.clone();
    let Value::Object(p) = patch else { return Value::Object(out) };
    for (key, current) in b {
        let Some(next) = p.get(key) else { continue };
        let v = match current {
            Value::Object(_) => merge_known(current, next),
            Value::Number(_) => match js::num(next) {
                Some(n) if n.is_finite() => jnum(n),
                _ => current.clone(),
            },
            Value::Bool(_) => json!(js::truthy(next)),
            Value::String(_) => json!(js::slice_chars(&if next.is_null() { String::new() } else { js::s(next) }, 2000)),
            _ => next.clone(),
        };
        out.insert(key.clone(), v);
    }
    Value::Object(out)
}

pub struct Config {
    file: PathBuf,
    pub data: RefCell<J>,
}

fn f(v: &J) -> f64 {
    js::num(v).unwrap_or(0.0)
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

impl Config {
    pub fn new(file: PathBuf) -> Self {
        let c = Self { file, data: RefCell::new(merge_known(&defaults(), &json!({}))) };
        c.load();
        c
    }

    pub fn load(&self) -> J {
        let saved = js::read_json(&self.file).unwrap_or(json!({}));
        *self.data.borrow_mut() = merge_known(&defaults(), &saved);
        self.sanitize();
        self.data.borrow().clone()
    }

    fn sanitize(&self) {
        let mut d = self.data.borrow_mut();
        {
            let p = &mut d["profile"];
            if !["tu", "vous"].contains(&js::gs(p, "address").as_str()) {
                p["address"] = json!("tu");
            }
            if tone(&js::gs(p, "tone")).is_none() {
                p["tone"] = json!("complice");
            }
            if verbosity(&js::gs(p, "verbosity")).is_none() {
                p["verbosity"] = json!("court");
            }
            p["humor"] = jnum(clamp(f(&p["humor"]).round(), 0.0, 2.0));
        }
        {
            let v = &mut d["voice"];
            if !["livekit", "classic"].contains(&js::gs(v, "backend").as_str()) {
                v["backend"] = json!("livekit");
            }
            let lk = &mut v["livekit"];
            if !["v1-mini", "multilingual", "vad"].contains(&js::gs(lk, "turnDetector").as_str()) {
                lk["turnDetector"] = json!("v1-mini");
            }
            if !["native", "renderer"].contains(&js::gs(lk, "capture").as_str()) {
                lk["capture"] = json!("native");
            }
            let min = clamp(f(&lk["minEndpointingDelay"]), 0.1, 3.0);
            lk["minEndpointingDelay"] = jnum(min);
            lk["maxEndpointingDelay"] = jnum(f(&lk["maxEndpointingDelay"]).min(8.0).max(min));
            lk["vadSilence"] = jnum(clamp(f(&lk["vadSilence"]), 0.15, 1.5));
            lk["minInterruptionDuration"] = jnum(clamp(f(&lk["minInterruptionDuration"]), 0.2, 3.0));
            lk["port"] = jnum(clamp(f(&lk["port"]), 1024.0, 65000.0).round());
            if !["turbo", "fr-distil", "fr"].contains(&js::gs(lk, "sttModel").as_str()) {
                lk["sttModel"] = json!("turbo");
            }
            lk["sttBeam"] = jnum(clamp(f(&lk["sttBeam"]), 1.0, 5.0).round());
        }
        {
            let tx = &mut d["tmux"];
            if !["wsl", "native"].contains(&js::gs(tx, "launcher").as_str()) {
                tx["launcher"] = json!("native");
            }
            if !crate::re!(r"^[\w.-]{1,40}$").is_match(&js::gs(tx, "session")) {
                tx["session"] = json!("jarvis");
            }
            if js::gs(tx, "claudeCommand").trim().is_empty() {
                tx["claudeCommand"] = json!("claude");
            }
            tx["enterDelayMs"] = jnum(clamp(f(&tx["enterDelayMs"]), 0.0, 3000.0).round());
        }
        {
            let root = crate::re!(r"[\\/]+$").replace(js::gs(&d["projects"], "root").trim(), "").to_string();
            d["projects"]["root"] = json!(if root.is_empty() { "E:\\www\\A-En cours".to_string() } else { root });
        }
        {
            let w = &mut d["widget"];
            w["size"] = jnum(clamp(f(&w["size"]), 56.0, 200.0).round());
            w["opacity"] = jnum(clamp(f(&w["opacity"]), 0.3, 1.0));
        }
        d["desk"]["monitor"] = jnum(clamp(f(&d["desk"]["monitor"]), 0.0, 8.0).round());
        d["desk"]["desktop"] = jnum(clamp(f(&d["desk"]["desktop"]), 1.0, 9.0).round());
        d["desk"]["wallpaperDim"] = jnum((clamp(f(&d["desk"]["wallpaperDim"]), 0.0, 0.9) * 100.0).round() / 100.0);
        let vol = js::num(&d["audio"]["jarvisVolume"]).filter(|v| v.is_finite()).unwrap_or(0.0);
        d["audio"]["jarvisVolume"] = jnum(clamp(vol, 0.0, 100.0).round());
        // Cadence : seules les valeurs proposées sont acceptées (la plus proche, sinon « libre »).
        let fps = f(&d["performance"]["maxFps"]).round() as i64;
        let fps = if MAX_FPS_CHOICES.contains(&fps) {
            fps
        } else {
            MAX_FPS_CHOICES.iter().copied().fold(0, |best, c| if (c - fps).abs() < (best - fps).abs() { c } else { best })
        };
        d["performance"]["maxFps"] = json!(fps);
        d["buildSpace"]["alertGb"] = jnum(clamp(f(&d["buildSpace"]["alertGb"]), 5.0, 2000.0).round());
    }

    pub fn save(&self) {
        let _ = js::write_json(&self.file, &self.data.borrow());
    }

    /// Copie d'une section (ou de tout).
    pub fn get(&self, section: &str) -> J {
        if section.is_empty() {
            self.data.borrow().clone()
        } else {
            self.data.borrow().get(section).cloned().unwrap_or(Value::Null)
        }
    }

    pub fn all(&self) -> J {
        self.data.borrow().clone()
    }

    /// Applique un correctif partiel et renvoie les sections modifiées.
    pub fn update(&self, patch: &J) -> Vec<String> {
        let before = self.data.borrow().clone();
        let next = merge_known(&before, patch);
        *self.data.borrow_mut() = next;
        self.sanitize();
        let mut changed = vec![];
        if let Value::Object(keys) = defaults() {
            let d = self.data.borrow();
            for key in keys.keys() {
                if before.get(key) != d.get(key) {
                    changed.push(key.clone());
                }
            }
        }
        if !changed.is_empty() {
            self.save();
        }
        changed
    }
}

// --- Profil de dialogue -------------------------------------------------------------------------

/// Nom sous lequel Jarvis s'adresse à l'utilisateur : réglage explicite, sinon prénom mémorisé.
pub fn address_name(profile: &J, memory_name: Option<&str>) -> String {
    let n = js::gs(profile, "name");
    let n = if n.is_empty() { memory_name.unwrap_or("").to_string() } else { n };
    n.trim().to_string()
}

/// Formules adaptées au tutoiement ou au vouvoiement.
#[derive(Clone)]
pub struct Forms {
    pub vous: bool,
    pub toi: &'static str,
    pub te: &'static str,
    pub ton: &'static str,
    pub tu: &'static str,
    pub peux: &'static str,
    pub veux: &'static str,
    pub souhaites: &'static str,
    pub dis: &'static str,
    pub saisis: &'static str,
    pub tape: &'static str,
    pub name: String,
    pub honorific: String,
    pub call: String,
}

impl Forms {
    /// Pronom complément devant un verbe, avec élision : « t'aider », « te donner », « vous aider ».
    pub fn cod(&self, verb: &str) -> String {
        if self.vous {
            format!("vous {verb}")
        } else if crate::re!(r"(?i)^[aeiouyhéèêàâîôû]").is_match(verb) {
            format!("t'{verb}")
        } else {
            format!("te {verb}")
        }
    }
}

pub fn forms(profile: &J) -> Forms {
    let vous = js::gs(profile, "address") == "vous";
    Forms {
        vous,
        toi: if vous { "vous" } else { "toi" },
        te: if vous { "vous" } else { "te" },
        ton: if vous { "votre" } else { "ton" },
        tu: if vous { "vous" } else { "tu" },
        peux: if vous { "pouvez" } else { "peux" },
        veux: if vous { "voulez" } else { "veux" },
        souhaites: if vous { "souhaitez" } else { "souhaites" },
        dis: if vous { "dites" } else { "dis" },
        saisis: if vous { "saisissez" } else { "saisis" },
        tape: if vous { "tapez" } else { "tape" },
        name: String::new(),
        honorific: String::new(),
        call: String::new(),
    }
}

/// Consigne de dialogue injectée dans le prompt système de Claude Code / AGY.
pub fn dialogue_instructions(profile: &J, memory_name: Option<&str>) -> String {
    let name = address_name(profile, memory_name);
    let honorific = js::gs(profile, "honorific");
    let mut lines: Vec<String> = vec![];
    lines.push(if js::gs(profile, "address") == "vous" { "Vouvoie l'utilisateur." } else { "Tutoie l'utilisateur." }.into());
    if !honorific.is_empty() && !name.is_empty() {
        lines.push(format!("Appelle-le « {honorific} » ou par son prénom, {name}, avec naturel."));
    } else if !honorific.is_empty() {
        lines.push(format!("Appelle-le « {honorific} »."));
    } else if !name.is_empty() {
        lines.push(format!("Son prénom est {name} ; utilise-le avec parcimonie."));
    }
    lines.push(tone(&js::gs(profile, "tone")).unwrap_or(tone("complice").unwrap()).into());
    let humor = js::num(&profile["humor"]).unwrap_or(1.0);
    lines.push(
        if humor == 0.0 {
            "Pas d'humour."
        } else if humor == 2.0 {
            "Tu peux faire de l'humour."
        } else {
            "Une touche d'humour légère est bienvenue quand elle s'y prête."
        }
        .into(),
    );
    lines.push(verbosity(&js::gs(profile, "verbosity")).unwrap_or(verbosity("court").unwrap()).into());
    let instr = js::gs(profile, "instructions");
    if !instr.trim().is_empty() {
        lines.push(format!("Consignes personnelles de l'utilisateur, à respecter : {}", instr.trim()));
    }
    lines.join(" ")
}

/// Résumé du profil inscrit dans la mémoire RAG (catégorie « dialogue »).
pub fn profile_summary(profile: &J) -> String {
    let mut parts: Vec<String> = vec![];
    parts.push(if js::gs(profile, "address") == "vous" { "préfère être vouvoyé" } else { "préfère être tutoyé" }.into());
    let h = js::gs(profile, "honorific");
    if !h.is_empty() {
        parts.push(format!("aime être appelé « {h} »"));
    }
    let n = js::gs(profile, "name");
    if !n.is_empty() {
        parts.push(format!("se fait appeler {n}"));
    }
    parts.push(format!("ton souhaité : {}", js::gs(profile, "tone")));
    let v = js::gs(profile, "verbosity");
    parts.push(format!("réponses {}", if v == "tres-court" { "très courtes" } else if v == "detaille" { "détaillées" } else { "courtes" }));
    let i = js::gs(profile, "instructions");
    if !i.trim().is_empty() {
        parts.push(format!("consignes : {}", i.trim()));
    }
    format!("Préférences de dialogue de l'utilisateur : {}.", parts.join(", "))
}

#[allow(dead_code)]
fn _unused(_: Map<String, Value>) {}
