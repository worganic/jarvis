// Mesure réelle de la consommation de Jarvis (core/metrics.js de la v3) : mémoire (working set) et
// CPU de chaque processus de l'arbre — coquille Tauri, pages WebView2, noyau, moteur vocal, démons IA.
// Alimente la page Optimisation et le journal de mesure ([metrics], une ligne par minute).
// La v3 passait par PowerShell/CIM (~0,5 s de CPU) ; ici l'API Windows est lue directement, en
// arrière-plan au plus toutes les 15 s.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use serde_json::json;

use crate::js::{self, J};

const REFRESH_MS: i64 = 15000;

/// Processus racines : coquille, noyau, moteur vocal.
#[derive(Clone, Copy, Default)]
pub struct Roots {
    pub shell: Option<u32>,
    pub core: u32,
    pub engine: Option<u32>,
}

fn label(name: &str, pid: u32, roots: &Roots) -> String {
    let n = name.to_lowercase();
    if pid == roots.core {
        return "noyau (Rust)".into();
    }
    if Some(pid) == roots.engine {
        return "moteur vocal (Rust)".into();
    }
    if Some(pid) == roots.shell {
        return "coquille (Tauri)".into();
    }
    if n.starts_with("msedgewebview2") {
        return "fenêtres (WebView2)".into();
    }
    if n.starts_with("claude") {
        return "IA · Claude Code".into();
    }
    if n.starts_with("agy") || n.contains("gemini") {
        return "IA · AGY".into();
    }
    if n.starts_with("python") {
        return "outils Python".into();
    }
    if n.starts_with("node") {
        return "outils Node (MCP)".into();
    }
    if n.starts_with("jarvis-core") {
        return "outils Jarvis (MCP, veilles)".into();
    }
    name.to_string()
}

struct Sample {
    mb: u64,
    cpu: f64,
    label: String,
}

pub struct Metrics {
    get_roots: Box<dyn Fn() -> Roots>,
    snapshot: Rc<RefCell<J>>,
    prev_cpu: Rc<RefCell<HashMap<u32, (u64, i64)>>>,
    busy: Rc<Cell<bool>>,
}

impl Metrics {
    pub fn new(get_roots: Box<dyn Fn() -> Roots>) -> Self {
        Self {
            get_roots,
            snapshot: Rc::new(RefCell::new(json!({ "mb": 0, "cpu": 0, "rows": [], "at": 0 }))),
            prev_cpu: Rc::new(RefCell::new(HashMap::new())),
            busy: Rc::new(Cell::new(false)),
        }
    }

    /// Nouvelle mesure en arrière-plan (fil séparé : l'énumération ne bloque pas le noyau).
    pub fn refresh(&self) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        let roots = (self.get_roots)();
        let prev: HashMap<u32, (u64, i64)> = self.prev_cpu.borrow().clone();
        let (snapshot, prev_cpu, busy) = (self.snapshot.clone(), self.prev_cpu.clone(), self.busy.clone());
        js::spawn(async move {
            let res = tokio::task::spawn_blocking(move || measure(roots, prev)).await;
            busy.set(false);
            if let Ok((snap, cpu)) = res {
                *snapshot.borrow_mut() = snap;
                *prev_cpu.borrow_mut() = cpu;
            }
        });
    }

    /// Lecture immédiate (administration) ; relance une mesure si l'instantané a vieilli.
    pub fn collect(&self) -> J {
        let at = self.snapshot.borrow()["at"].as_i64().unwrap_or(0);
        if js::now_ms() - at > REFRESH_MS {
            self.refresh();
        }
        self.snapshot.borrow().clone()
    }
}

fn measure(roots: Roots, prev: HashMap<u32, (u64, i64)>) -> (J, HashMap<u32, (u64, i64)>) {
    let list = jarvis_native::window::processes();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut names: HashMap<u32, String> = HashMap::new();
    for (pid, ppid, name) in &list {
        children.entry(*ppid).or_default().push(*pid);
        names.insert(*pid, name.clone());
    }
    // Arbre de la coquille (ou du noyau seul). Parcours sans récursion et sans repasser par un même
    // processus : Windows réutilise les numéros, un « parent » disparu peut porter celui d'un descendant.
    let top = roots.shell.unwrap_or(roots.core);
    let mut seen = HashSet::new();
    let mut stack = vec![(top, false)];
    let mut mine: Vec<(u32, bool)> = vec![];
    while let Some((pid, in_engine)) = stack.pop() {
        if pid == 0 || !seen.insert(pid) {
            continue;
        }
        let engine = in_engine || Some(pid) == roots.engine;
        if names.contains_key(&pid) {
            mine.push((pid, engine));
        }
        for c in children.get(&pid).into_iter().flatten() {
            stack.push((*c, engine));
        }
    }
    let now = js::now_ms();
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
    let mut cpu_state = HashMap::new();
    let mut groups: Vec<Sample> = vec![];
    for (pid, engine) in mine {
        let Some((ws, cpu100ns)) = jarvis_native::window::process_stats(pid) else { continue };
        let cpu_ms = cpu100ns / 10000;
        let cpu = match prev.get(&pid) {
            Some((ms, at)) if now > *at => ((cpu_ms.saturating_sub(*ms)) as f64 / (now - at) as f64 / cpus * 100.0).max(0.0),
            _ => 0.0,
        };
        cpu_state.insert(pid, (cpu_ms, now));
        let key = if engine { "moteur vocal (Rust)".to_string() } else { label(&names[&pid], pid, &roots) };
        match groups.iter_mut().find(|g| g.label == key) {
            Some(g) => {
                g.mb += ws / 1_048_576;
                g.cpu += cpu;
            }
            None => groups.push(Sample { mb: ws / 1_048_576, cpu, label: key }),
        }
    }
    groups.sort_by(|a, b| b.mb.cmp(&a.mb));
    let rows: Vec<J> = groups.iter().map(|g| json!({ "label": g.label, "mb": g.mb, "cpu": js::round1(g.cpu) })).collect();
    let snap = json!({
        "mb": groups.iter().map(|g| g.mb).sum::<u64>(),
        "cpu": js::round1(groups.iter().map(|g| js::round1(g.cpu)).sum::<f64>()),
        "rows": rows,
        "at": now
    });
    (snap, cpu_state)
}
