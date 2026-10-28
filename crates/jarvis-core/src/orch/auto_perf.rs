// Mode automatique de l'optimisation (orchestrator/auto-perf.js) : réduit temporairement certaines
// options quand la machine est chargée, et les rétablit quand elle se calme. Ne touche jamais aux
// réglages enregistrés : calcule des réductions posées par-dessus. Lissage sur 30 s, maintien,
// hystérésis, délai mini entre deux changements, frein après des rebonds.

use std::cell::RefCell;

use serde_json::{json, Map, Value};

use crate::js::{self, J};

const WINDOW_MS: i64 = 30 * 1000;
const UP_HOLD_MS: i64 = 20 * 1000;
const DOWN_HOLD_MS: i64 = 60 * 1000;
const COOLDOWN_MS: i64 = 45 * 1000;
const MIN_SAMPLES: usize = 5;
const REBOUND_MS: i64 = 3 * 60 * 1000;
const MAX_DOWN_HOLD_MS: i64 = 5 * 60 * 1000;
const BACKOFF_RESET_MS: i64 = 10 * 60 * 1000;

struct ChannelDef {
    up: &'static [f64],
    down: &'static [f64],
    tiers: fn() -> Vec<J>,
}

fn cpu_tiers() -> Vec<J> {
    vec![
        json!({ "maxFps": 30, "metricsLog": false }),
        json!({ "orbDetail": false, "hudBackground": false, "audioDeviceWatch": false }),
        json!({ "cards": false, "voiceWaveform": false, "maxFps": 20 }),
    ]
}

fn ram_tiers() -> Vec<J> {
    vec![json!({ "semanticMemory": false }), json!({ "hudBackground": false, "orbDetail": false })]
}

const CPU: ChannelDef = ChannelDef { up: &[70.0, 80.0, 90.0], down: &[55.0, 68.0, 78.0], tiers: cpu_tiers };
const RAM: ChannelDef = ChannelDef { up: &[82.0, 90.0], down: &[72.0, 82.0], tiers: ram_tiers };

pub fn label(key: &str) -> &str {
    match key {
        "maxFps" => "cadence d’affichage",
        "metricsLog" => "journal de mesure",
        "orbDetail" => "cœur détaillé",
        "hudBackground" => "circuit du fond",
        "audioDeviceWatch" => "surveillance audio",
        "cards" => "cards et branches",
        "voiceWaveform" => "couronne vocale",
        "semanticMemory" => "mémoire sémantique",
        k => k,
    }
}

struct Channel {
    def: &'static ChannelDef,
    level: usize,
    samples: Vec<(i64, f64)>,
    over_since: Option<i64>,
    under_since: Option<i64>,
    last_change: i64,
    last_down_at: i64,
    down_hold: i64,
}

impl Channel {
    fn new(def: &'static ChannelDef) -> Self {
        Self { def, level: 0, samples: vec![], over_since: None, under_since: None, last_change: i64::MIN / 2, last_down_at: i64::MIN / 2, down_hold: DOWN_HOLD_MS }
    }

    /// Ajoute une mesure ; +1 / -1 si le niveau vient de monter / descendre.
    fn push(&mut self, value: f64, now: i64) -> i32 {
        if !value.is_finite() {
            return 0;
        }
        self.samples.push((now, value));
        self.samples.retain(|(t, _)| now - t <= WINDOW_MS);
        if self.samples.len() < MIN_SAMPLES {
            return 0;
        }
        if self.down_hold > DOWN_HOLD_MS && now - self.last_change > BACKOFF_RESET_MS {
            self.down_hold = DOWN_HOLD_MS;
        }
        let avg = self.samples.iter().map(|(_, v)| v).sum::<f64>() / self.samples.len() as f64;
        let can_up = self.level < self.def.up.len();
        let can_down = self.level > 0;
        if can_up && avg >= self.def.up[self.level] {
            self.over_since.get_or_insert(now);
        } else {
            self.over_since = None;
        }
        if can_down && avg <= self.def.down[self.level - 1] {
            self.under_since.get_or_insert(now);
        } else {
            self.under_since = None;
        }
        if now - self.last_change < COOLDOWN_MS {
            return 0;
        }
        if let Some(o) = self.over_since {
            if now - o >= UP_HOLD_MS {
                if now - self.last_down_at < REBOUND_MS {
                    self.down_hold = (self.down_hold * 2).min(MAX_DOWN_HOLD_MS);
                }
                self.level += 1;
                self.last_change = now;
                self.over_since = None;
                self.under_since = None;
                return 1;
            }
        }
        if let Some(u) = self.under_since {
            if now - u >= self.down_hold {
                self.level -= 1;
                self.last_down_at = now;
                self.last_change = now;
                self.over_since = None;
                self.under_since = None;
                return -1;
            }
        }
        0
    }

    fn actions(&self) -> Map<String, Value> {
        let mut out = Map::new();
        let tiers = (self.def.tiers)();
        for t in tiers.iter().take(self.level) {
            for (k, v) in t.as_object().unwrap() {
                let nv = if k == "maxFps" && out.get(k).map(js::truthy).unwrap_or(false) {
                    json!(out[k].as_f64().unwrap_or(0.0).min(v.as_f64().unwrap_or(0.0)) as i64)
                } else {
                    v.clone()
                };
                out.insert(k.clone(), nv);
            }
        }
        out
    }
}

pub struct AutoPerf {
    cpu: RefCell<Channel>,
    ram: RefCell<Channel>,
}

impl Default for AutoPerf {
    fn default() -> Self {
        Self { cpu: RefCell::new(Channel::new(&CPU)), ram: RefCell::new(Channel::new(&RAM)) }
    }
}

impl AutoPerf {
    /// Une mesure de télémétrie ({ cpu, ramPct }) ; true si un niveau a changé.
    pub fn sample(&self, telemetry: &J) -> bool {
        let now = js::now_ms();
        let a = self.cpu.borrow_mut().push(js::num(&telemetry["cpu"]).unwrap_or(f64::NAN), now);
        let b = self.ram.borrow_mut().push(js::num(&telemetry["ramPct"]).unwrap_or(f64::NAN), now);
        a != 0 || b != 0
    }

    pub fn state(&self) -> J {
        json!({ "cpuLevel": self.cpu.borrow().level, "ramLevel": self.ram.borrow().level })
    }

    /// Réglages effectifs : { settings, overridden, labels }.
    pub fn effective(&self, base: &J) -> J {
        let mut actions = self.cpu.borrow().actions();
        for (k, v) in self.ram.borrow().actions() {
            let nv = if k == "maxFps" && actions.get(&k).map(js::truthy).unwrap_or(false) { json!(actions[&k].as_f64().unwrap_or(0.0).min(v.as_f64().unwrap_or(0.0)) as i64) } else { v };
            actions.insert(k, nv);
        }
        let mut settings = base.clone();
        let mut overridden: Vec<String> = vec![];
        for (k, v) in actions {
            if k == "maxFps" {
                let bf = base["maxFps"].as_f64().unwrap_or(0.0);
                let vf = v.as_f64().unwrap_or(0.0);
                let next = if bf == 0.0 { vf } else { bf.min(vf) };
                if next != bf {
                    settings["maxFps"] = json!(next as i64);
                    overridden.push(k);
                }
            } else if base[&k] == json!(true) && v == json!(false) {
                settings[&k] = json!(false);
                overridden.push(k);
            }
        }
        let labels: Vec<String> = overridden.iter().map(|k| label(k).to_string()).collect();
        json!({ "settings": settings, "overridden": overridden, "labels": labels })
    }
}

/// Description lisible des seuils et des délais, pour la page Optimisation.
pub fn describe() -> J {
    let fmt = |k: &str, v: &J| if k == "maxFps" { format!("cadence {} images/s", js::s(v)) } else { label(k).to_string() };
    let ch = |d: &ChannelDef| {
        json!({
            "up": d.up, "down": d.down,
            "tiers": (d.tiers)().iter().map(|t| t.as_object().unwrap().iter().map(|(k, v)| fmt(k, v)).collect::<Vec<_>>()).collect::<Vec<_>>()
        })
    };
    json!({
        "timings": { "windowS": WINDOW_MS / 1000, "upHoldS": UP_HOLD_MS / 1000, "downHoldS": DOWN_HOLD_MS / 1000, "cooldownS": COOLDOWN_MS / 1000, "maxDownHoldS": MAX_DOWN_HOLD_MS / 1000 },
        "channels": { "cpu": ch(&CPU), "ram": ch(&RAM) }
    })
}
