// Compteur de consommation du jour (orchestrator/usage-daily.js) : appels aux IA externes, tokens
// entrants / sortants, réponses traitées localement. Remis à zéro à minuit (heure locale) ; les 30
// derniers jours sont conservés dans data/usage-daily.json.

use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::js::{self, J};

const KEEP_DAYS: usize = 30;

pub fn day_key() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn empty_totals() -> J {
    json!({ "calls": 0, "input": 0, "cacheRead": 0, "cacheWrite": 0, "output": 0, "estimated": 0 })
}

fn empty_day() -> J {
    let mut d = empty_totals();
    d["local"] = json!(0);
    d["ignored"] = json!(0);
    d["engines"] = json!({});
    d
}

fn num(v: &J) -> i64 {
    match js::num(v) {
        Some(f) if f.is_finite() => f.round().max(0.0) as i64,
        _ => 0,
    }
}

fn inc(v: &mut J, key: &str, by: i64) {
    let n = v[key].as_i64().unwrap_or(0);
    v[key] = json!(n + by);
}

pub struct UsageDaily {
    file: PathBuf,
    days: RefCell<Map<String, Value>>,
}

impl UsageDaily {
    pub fn new(file: PathBuf) -> Self {
        let u = Self { file, days: RefCell::new(Map::new()) };
        if let Some(saved) = js::read_json(&u.file) {
            if let Some(Value::Object(d)) = saved.get("days") {
                *u.days.borrow_mut() = d.clone();
            }
        }
        u
    }

    fn save(&self) {
        let mut days = self.days.borrow_mut();
        let mut keys: Vec<String> = days.keys().cloned().collect();
        keys.sort();
        for old in keys.iter().take(keys.len().saturating_sub(KEEP_DAYS)) {
            days.remove(old);
        }
        let _ = js::write_json(&self.file, &json!({ "days": Value::Object(days.clone()) }));
    }

    fn with_day<R>(&self, f: impl FnOnce(&mut J) -> R) -> R {
        let key = day_key();
        let mut days = self.days.borrow_mut();
        let day = days.entry(key).or_insert_with(empty_day);
        f(day)
    }

    pub fn add_call(&self, engine: &str) {
        self.with_day(|d| {
            inc(d, "calls", 1);
            let e = d["engines"].as_object_mut().unwrap().entry(engine.to_string()).or_insert_with(empty_totals);
            inc(e, "calls", 1);
        });
        self.save();
    }

    /// usage : { input, cacheRead, cacheWrite, output } ; estimated : chiffres calculés par Jarvis.
    pub fn add_tokens(&self, engine: &str, usage: &J, estimated: bool) {
        let vals = [
            ("input", num(&usage["input"])),
            ("cacheRead", num(&usage["cacheRead"])),
            ("cacheWrite", num(&usage["cacheWrite"])),
            ("output", num(&usage["output"])),
        ];
        self.with_day(|d| {
            for (k, v) in vals {
                inc(d, k, v);
            }
            if estimated {
                inc(d, "estimated", 1);
            }
            let e = d["engines"].as_object_mut().unwrap().entry(engine.to_string()).or_insert_with(empty_totals);
            for (k, v) in vals {
                inc(e, k, v);
            }
            if estimated {
                inc(e, "estimated", 1);
            }
        });
        self.save();
    }

    /// kind : « local » (intention exécutée sans IA) ou « ignored » (bruit, fragment écarté).
    pub fn add_local(&self, kind: &str) {
        self.with_day(|d| inc(d, if kind == "ignored" { "ignored" } else { "local" }, 1));
        self.save();
    }

    pub fn snapshot(&self) -> J {
        self.with_day(|d| {
            let g = |k: &str| d[k].as_i64().unwrap_or(0);
            json!({
                "date": day_key(),
                "calls": g("calls"),
                "local": g("local"),
                "ignored": g("ignored"),
                "in": g("input") + g("cacheRead") + g("cacheWrite"),
                "cacheRead": g("cacheRead"),
                "out": g("output"),
                "estimated": g("estimated") > 0,
                "engines": d["engines"].clone()
            })
        })
    }

    pub fn history(&self) -> Vec<J> {
        let days = self.days.borrow();
        let mut keys: Vec<&String> = days.keys().collect();
        keys.sort();
        keys.into_iter().map(|k| js::spread(&json!({ "date": k }), &days[k])).collect()
    }
}

/// Relevé de tokens d'un événement de fin de tour, quel que soit le moteur (Claude Code, Gemini…).
pub fn extract_usage(event: &J) -> Option<J> {
    let mut queue: std::collections::VecDeque<&J> = std::collections::VecDeque::new();
    queue.push_back(event);
    let first = |o: &J, keys: &[&str]| -> Option<J> { keys.iter().find_map(|k| o.get(*k).filter(|v| !v.is_null()).cloned()) };
    while let Some(obj) = queue.pop_front() {
        let Value::Object(map) = obj else { continue };
        let input = first(obj, &["input_tokens", "inputTokens", "prompt_tokens", "promptTokenCount", "prompt_token_count"]);
        let output = first(obj, &["output_tokens", "outputTokens", "completion_tokens", "candidatesTokenCount", "candidates_token_count"]);
        if input.is_some() || output.is_some() {
            let cache_read = num(&first(obj, &["cache_read_input_tokens", "cachedContentTokenCount", "cached_content_token_count"]).unwrap_or(Value::Null));
            let gemini = map.contains_key("promptTokenCount") || map.contains_key("prompt_token_count");
            let inp = num(&input.unwrap_or(Value::Null));
            let thoughts = num(&first(obj, &["thoughtsTokenCount", "thoughts_token_count"]).unwrap_or(Value::Null));
            return Some(json!({
                "input": if gemini { (inp - cache_read).max(0) } else { inp },
                "output": num(&output.unwrap_or(Value::Null)) + thoughts,
                "cacheRead": cache_read,
                "cacheWrite": num(&obj["cache_creation_input_tokens"])
            }));
        }
        for v in map.values() {
            if v.is_object() || v.is_array() {
                if let Value::Array(a) = v {
                    for x in a {
                        queue.push_back(x);
                    }
                } else {
                    queue.push_back(v);
                }
            }
        }
    }
    None
}

/// Estimation grossière (≈ 4 caractères par token en français).
pub fn estimate_tokens(text: &str) -> i64 {
    (text.encode_utf16().count() as f64 / 4.0).ceil() as i64
}
