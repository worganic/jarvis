// Outils communs du portage : les idiomes JavaScript de la v3 (valeurs JSON dynamiques, vérité,
// minuteries setTimeout/setInterval sur la boucle locale, normalisation du texte), pour que chaque
// module garde la logique et la lecture de son original.

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;

pub type J = Value;
pub type Obj = Map<String, Value>;

/// Expression régulière compilée une seule fois (`re!(r"(?i)^bonjour")`).
#[macro_export]
macro_rules! re {
    ($pat:expr) => {{
        static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new($pat).unwrap());
        &*RE
    }};
}

/// Expression régulière avec regards en arrière/avant (fancy-regex), compilée une seule fois.
#[macro_export]
macro_rules! fre {
    ($pat:expr) => {{
        static RE: std::sync::LazyLock<fancy_regex::Regex> = std::sync::LazyLock::new(|| fancy_regex::Regex::new($pat).unwrap());
        &*RE
    }};
}

/// Test d'une expression fancy-regex (une erreur de recherche vaut « pas de correspondance »).
pub fn ftest(re: &fancy_regex::Regex, text: &str) -> bool {
    re.is_match(text).unwrap_or(false)
}

/// Date.now()
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// new Date().toISOString()
pub fn iso_now() -> String {
    iso_of(now_ms())
}

pub fn iso_of(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Date.parse(iso) → millisecondes (NaN → None).
pub fn parse_date_ms(s: &str) -> Option<i64> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(d.timestamp_millis());
    }
    if let Ok(d) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return chrono::Local.from_local_datetime(&d).single().map(|d| d.timestamp_millis());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        // Date seule : UTC, comme en JS.
        return Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis());
    }
    None
}

use chrono::TimeZone;

/// Vérité JavaScript d'une valeur JSON.
pub fn truthy(v: &J) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// Champ d'un objet JSON (« undefined » → Null).
pub fn get<'a>(v: &'a J, key: &str) -> &'a J {
    static NULL: J = Value::Null;
    v.get(key).unwrap_or(&NULL)
}

/// String(x || '') : chaîne d'une valeur, vide si fausse.
pub fn s(v: &J) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(false) => String::new(),
        Value::Number(n) => num_str(n.as_f64().unwrap_or(0.0)),
        Value::Bool(true) => "true".into(),
        other => other.to_string(),
    }
}

/// String(x) d'un nombre JS (entiers sans « .0 »).
pub fn num_str(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

/// Champ chaîne d'un objet (vide si absent ou non chaîne).
pub fn gs(v: &J, key: &str) -> String {
    s(get(v, key))
}

pub fn gb(v: &J, key: &str) -> bool {
    truthy(get(v, key))
}

/// Number(x) (NaN → None).
pub fn num(v: &J) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                Some(0.0)
            } else {
                t.parse::<f64>().ok()
            }
        }
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Null => Some(0.0),
        _ => None,
    }
}

pub fn gn(v: &J, key: &str) -> Option<f64> {
    match v.get(key) {
        None => None,
        Some(x) => num(x),
    }
}

/// Number(x) || d
pub fn gn_or(v: &J, key: &str, d: f64) -> f64 {
    match gn(v, key) {
        Some(f) if f != 0.0 && f.is_finite() => f,
        _ => d,
    }
}

/// Fusion superficielle d'objets ({ ...a, ...b }).
pub fn spread(a: &J, b: &J) -> J {
    let mut out = match a {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    if let Value::Object(m) = b {
        for (k, v) in m {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// Ajoute les champs de `b` à l'objet `a` (Object.assign).
pub fn assign(a: &mut J, b: J) {
    if !a.is_object() {
        *a = Value::Object(Map::new());
    }
    if let (Value::Object(m), Value::Object(n)) = (a, b) {
        for (k, v) in n {
            m.insert(k, v);
        }
    }
}

/// Retire les champs « undefined » d'un objet construit avec des valeurs facultatives.
pub fn compact(v: J) -> J {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(_, v)| !v.is_null()).collect()),
        other => other,
    }
}

/// Minuscules sans accents (normalize('NFD').replace(/[̀-ͯ]/g, '')).
pub fn strip_accents(text: &str) -> String {
    text.nfd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).collect()
}

pub fn fold_lower(text: &str) -> String {
    strip_accents(&text.to_lowercase())
}

/// Tronque à `max` caractères (les chaînes JS comptent en UTF-16 ; ici en caractères).
pub fn slice_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// Premier caractère en majuscule.
pub fn capitalize(text: &str) -> String {
    let mut c = text.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Entier aléatoire dans [0, n).
pub fn rand_below(n: usize) -> usize {
    use rand::Rng;
    if n == 0 {
        0
    } else {
        rand::thread_rng().gen_range(0..n)
    }
}

pub fn round1(f: f64) -> f64 {
    (f * 10.0).round() / 10.0
}

pub fn round2(f: f64) -> f64 {
    (f * 100.0).round() / 100.0
}

// --- Boucle d'événements locale ---------------------------------------------------------------------

/// Lance une tâche sur la boucle locale (équivalent d'une promesse non attendue).
pub fn spawn<F: Future<Output = ()> + 'static>(fut: F) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_local(fut)
}

pub async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await
}

/// Minuterie annulable (setTimeout / setInterval). Annulée à `clear()` ou quand elle est abandonnée.
#[derive(Default)]
pub struct Timer(RefCell<Option<tokio::task::JoinHandle<()>>>);

impl Timer {
    pub fn new() -> Self {
        Self(RefCell::new(None))
    }

    /// setTimeout : remplace la minuterie précédente.
    pub fn set<F: FnOnce() + 'static>(&self, ms: u64, f: F) {
        self.clear();
        let h = spawn(async move {
            sleep_ms(ms).await;
            f();
        });
        *self.0.borrow_mut() = Some(h);
    }

    /// setInterval : `f` toutes les `ms` millisecondes.
    pub fn every<F: FnMut() + 'static>(&self, ms: u64, mut f: F) {
        self.clear();
        let h = spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(ms.max(1)));
            iv.tick().await; // le premier tic est immédiat : setInterval attend une période
            loop {
                iv.tick().await;
                f();
            }
        });
        *self.0.borrow_mut() = Some(h);
    }

    pub fn clear(&self) {
        if let Some(h) = self.0.borrow_mut().take() {
            h.abort();
        }
    }

    pub fn active(&self) -> bool {
        self.0.borrow().as_ref().map(|h| !h.is_finished()).unwrap_or(false)
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.clear();
    }
}

/// setTimeout « jeté » : non annulable.
pub fn set_timeout<F: FnOnce() + 'static>(ms: u64, f: F) {
    spawn(async move {
        sleep_ms(ms).await;
        f();
    });
}

/// Rc<RefCell<T>> raccourci.
pub type Shared<T> = Rc<RefCell<T>>;

pub fn shared<T>(v: T) -> Shared<T> {
    Rc::new(RefCell::new(v))
}

// --- Fichiers JSON ----------------------------------------------------------------------------------

pub fn read_json(path: &std::path::Path) -> Option<J> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// JSON.stringify(v, null, 2)
pub fn pretty(v: &J) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| "null".into())
}

pub fn write_json(path: &std::path::Path, v: &J) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, pretty(v))
}

/// Écriture atomique (fichier temporaire puis renommage).
pub fn write_json_atomic(path: &std::path::Path, v: &J) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, pretty(v))?;
    std::fs::rename(&tmp, path)
}

pub fn arr(v: &J) -> Vec<J> {
    v.as_array().cloned().unwrap_or_default()
}

pub fn str_list(v: &J) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Les deux premières positions d'un groupe capturé (`m[1]` en JS, « undefined » → None).
pub fn cap<'a>(c: &'a regex::Captures, i: usize) -> Option<&'a str> {
    c.get(i).map(|m| m.as_str())
}

pub fn fcap<'a>(c: &'a fancy_regex::Captures, i: usize) -> Option<&'a str> {
    c.get(i).map(|m| m.as_str())
}

/// Date locale au format fr-FR.
pub fn local_now() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now()
}

pub fn local_of(ms: i64) -> chrono::DateTime<chrono::Local> {
    chrono::Local.timestamp_millis_opt(ms).single().unwrap_or_else(chrono::Local::now)
}

pub const MOIS: [&str; 12] = [
    "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre",
];
pub const JOURS: [&str; 7] = ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"];

/// toLocaleString('fr-FR') : « 13/10/2026 18:39:05 »
pub fn fr_datetime(ms: i64) -> String {
    local_of(ms).format("%d/%m/%Y %H:%M:%S").to_string()
}

/// toLocaleDateString('fr-FR', { weekday: 'long', day: 'numeric', month: 'long' })
pub fn fr_long_date(d: &chrono::DateTime<chrono::Local>) -> String {
    use chrono::Datelike;
    format!("{} {} {}", JOURS[d.weekday().num_days_from_sunday() as usize], d.day(), MOIS[d.month0() as usize])
}

/// Écrit une ligne dans un fichier (création au besoin), sans erreur visible.
pub fn append_line(path: &std::path::Path, line: &str) {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

pub fn home_dir() -> std::path::PathBuf {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).map(std::path::PathBuf::from).unwrap_or_default()
}

pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

// --- Événements (EventEmitter) -----------------------------------------------------------------------

pub type Listener = Rc<dyn Fn(&J)>;

/// Abonnés à des événements nommés (EventEmitter de Node).
#[derive(Default)]
pub struct Emitter {
    listeners: RefCell<Vec<(String, Listener)>>,
}

impl Emitter {
    pub fn on(&self, event: &str, f: impl Fn(&J) + 'static) {
        self.listeners.borrow_mut().push((event.to_string(), Rc::new(f)));
    }

    pub fn emit(&self, event: &str, data: &J) {
        let list: Vec<Listener> = self.listeners.borrow().iter().filter(|(e, _)| e == event).map(|(_, f)| f.clone()).collect();
        for f in list {
            f(data);
        }
    }
}
