// Couleurs des terminaux : chaque onglet de Windows Terminal reçoit une couleur distincte, posée dans
// l'onglet lui-même (séquence DECAC, voir jarvis-native::console) et reprise par sa ligne dans la card
// Bureau et par sa mini-card Claude Code dans la card Terminal. La couleur affichée dans les cards est celle
// que l'onglet montre vraiment (lue sur l'image de la fenêtre) : une couleur choisie à la main dans Windows
// Terminal, prioritaire sur celle de Jarvis, est donc reprise telle quelle. Les onglets ouverts avant Jarvis sont
// colorés à son démarrage. Les fenêtres console classiques (powershell.exe, cmd.exe sans Windows Terminal)
// n'ont pas d'onglet : seule leur ligne de la card Bureau est colorée.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use serde_json::{json, Value};

use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

/// Palette Campbell (jeu de couleurs par défaut de Windows Terminal) : (index de l'onglet, couleur affichée).
/// Ordre d'attribution : les plus contrastées d'abord.
const PALETTE: &[(u8, &str)] = &[
    (12, "#3B78FF"),
    (10, "#16C60C"),
    (13, "#B4009E"),
    (14, "#61D6D6"),
    (9, "#E74856"),
    (11, "#F9F1A5"),
    (6, "#3A96DD"),
    (2, "#13A10E"),
    (5, "#881798"),
    (3, "#C19C00"),
    (4, "#0037DA"),
    (1, "#C50F1F"),
];

fn hex_of(index: u8) -> &'static str {
    PALETTE.iter().find(|(i, _)| *i == index).map(|(_, h)| *h).unwrap_or("#3B78FF")
}

#[derive(Clone)]
struct Tab {
    color: u8,
    hwnd: i64,
    /// Couleur réellement affichée par l'onglet (« #RRGGBB »), si elle a pu être lue.
    shown: Option<String>,
}

impl Tab {
    fn hex(&self) -> String {
        self.shown.clone().unwrap_or_else(|| hex_of(self.color).to_string())
    }
}

pub struct TerminalColors {
    o: OrchRef,
    me: RefCell<Weak<TerminalColors>>,
    log: Rc<dyn Fn(String)>,
    timer: Timer,
    /// Couleurs déjà posées (pid de l'onglet → index) : reprises après une relance de Jarvis.
    file: PathBuf,
    /// Onglets de Windows Terminal : pid du processus de l'onglet (shell) → couleur et fenêtre hôte.
    tabs: RefCell<HashMap<u32, Tab>>,
    /// Fenêtres console classiques (hwnd) → couleur.
    consoles: RefCell<HashMap<i64, u8>>,
    /// Relevé des processus : pid → parent (pour retrouver l'onglet d'un Claude Code).
    parents: RefCell<HashMap<u32, u32>>,
    last_full: Cell<i64>,
    busy: Cell<bool>,
}

impl TerminalColors {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let me = Rc::new(Self {
            o,
            me: RefCell::new(Weak::new()),
            log,
            timer: Timer::new(),
            file,
            tabs: RefCell::new(HashMap::new()),
            consoles: RefCell::new(HashMap::new()),
            parents: RefCell::new(HashMap::new()),
            last_full: Cell::new(0),
            busy: Cell::new(false),
        });
        *me.me.borrow_mut() = Rc::downgrade(&me);
        me
    }

    pub fn start(&self) {
        let me = self.me.borrow().clone();
        if let Some(m) = me.upgrade() {
            m.scan();
        }
        self.timer.every(2500, move || {
            if let Some(m) = me.upgrade() {
                m.scan();
            }
        });
    }

    pub fn stop(&self) {
        self.timer.clear();
    }

    fn saved(&self) -> HashMap<u32, u8> {
        js::read_json(&self.file)
            .and_then(|d| d.as_object().cloned())
            .map(|m| m.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_u64()? as u8))).collect())
            .unwrap_or_default()
    }

    fn save(&self) {
        let m: serde_json::Map<String, Value> = self.tabs.borrow().iter().map(|(p, t)| (p.to_string(), json!(t.color))).collect();
        let _ = js::write_json(&self.file, &Value::Object(m));
    }

    /// Couleur libre : la première de la palette qu'aucun onglet ni aucune console n'utilise.
    fn pick(&self, taken: &HashSet<u8>) -> u8 {
        let n = PALETTE.len();
        PALETTE.iter().map(|(i, _)| *i).find(|i| !taken.contains(i)).unwrap_or(PALETTE[taken.len() % n].0)
    }

    fn used(&self) -> HashSet<u8> {
        self.tabs.borrow().values().map(|t| t.color).chain(self.consoles.borrow().values().copied()).collect()
    }

    /// Relevé : nouveaux onglets colorés, onglets fermés oubliés, fenêtres hôtes mises à jour (un onglet
    /// peut passer d'une fenêtre à l'autre) au plus toutes les 15 s.
    fn scan(&self) {
        let procs = jarvis_native::window::processes();
        *self.parents.borrow_mut() = procs.iter().map(|(p, pp, _)| (*p, *pp)).collect();
        let wt: HashSet<u32> = procs.iter().filter(|(_, _, n)| n.eq_ignore_ascii_case("WindowsTerminal.exe")).map(|(p, _, _)| *p).collect();
        let live: Vec<u32> = procs
            .iter()
            .filter(|(_, pp, n)| wt.contains(pp) && !n.eq_ignore_ascii_case("OpenConsole.exe") && !n.eq_ignore_ascii_case("conhost.exe"))
            .map(|(p, _, _)| *p)
            .collect();
        let mut changed = false;
        {
            let alive: HashSet<u32> = live.iter().copied().collect();
            let mut tabs = self.tabs.borrow_mut();
            let before = tabs.len();
            tabs.retain(|p, _| alive.contains(p));
            changed |= tabs.len() != before;
        }
        changed |= self.sync_consoles();
        let new: Vec<u32> = live.iter().copied().filter(|p| !self.tabs.borrow().contains_key(p)).collect();
        let now = js::now_ms();
        // Relevé complet (fenêtres hôtes, couleurs affichées) toutes les 6 s : une couleur changée à la main
        // dans Windows Terminal arrive vite dans les cards.
        let full = now - self.last_full.get() > 6000;
        if changed {
            self.save();
            self.refresh_cards();
        }
        if self.busy.get() || (new.is_empty() && !full) || live.is_empty() {
            return;
        }
        // Nouveaux onglets : couleur reprise du dernier lancement de Jarvis, sinon une couleur libre.
        let saved = self.saved();
        let mut taken = self.used();
        let mut colors: HashMap<u32, u8> = HashMap::new();
        for p in &new {
            let c = saved.get(p).copied().filter(|c| !taken.contains(c) && PALETTE.iter().any(|(i, _)| i == c)).unwrap_or_else(|| self.pick(&taken));
            taken.insert(c);
            colors.insert(*p, c);
        }
        let args: Vec<String> = live.iter().map(|p| match colors.get(p) {
            Some(c) => format!("{p}:{c}"),
            None => p.to_string(),
        }).collect();
        self.busy.set(true);
        self.last_full.set(now);
        let me = self.me.borrow().clone();
        js::spawn(async move {
            let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let mut full_args = vec!["console-tabs".to_string()];
            full_args.extend(args);
            let out = crate::proc::run(&exe, &full_args, None, 10000).await;
            let Some(m) = me.upgrade() else { return };
            m.busy.set(false);
            let list = match out {
                Ok(o) => serde_json::from_str::<J>(o.stdout.trim()).ok().map(|v| js::arr(&v)).unwrap_or_default(),
                Err(e) => {
                    (m.log)(format!("[terminaux] couleurs des onglets : {e}"));
                    return;
                }
            };
            let mut changed = false;
            {
                let mut tabs = m.tabs.borrow_mut();
                for r in &list {
                    let Some(pid) = r["pid"].as_u64().map(|p| p as u32) else { continue };
                    let hwnd = r["hwnd"].as_i64().unwrap_or(0);
                    let shown = r["shown"].as_str().map(String::from);
                    let exact = js::gb(r, "exact");
                    match (tabs.get_mut(&pid), colors.get(&pid)) {
                        (Some(t), _) => {
                            if t.hwnd != hwnd {
                                t.hwnd = hwnd;
                                changed = true;
                            }
                            // Couleur exacte (onglet sélectionné) : retenue. Estimée (onglet inactif, peint atténué) :
                            // seulement si rien de mieux n'est connu. Fenêtre réduite : la dernière couleur vue reste.
                            if shown.is_some() && t.shown != shown && (exact || t.shown.is_none()) {
                                t.shown = shown;
                                changed = true;
                            }
                        }
                        (None, Some(c)) => {
                            tabs.insert(pid, Tab { color: *c, hwnd, shown });
                            changed = true;
                        }
                        _ => {}
                    }
                }
            }
            if changed {
                m.save();
                m.refresh_cards();
            }
        });
    }

    /// Fenêtres console classiques : une couleur chacune, tant qu'elles sont ouvertes.
    fn sync_consoles(&self) -> bool {
        let o = self.o.up();
        let wins: Vec<i64> = o
            .desktop_state
            .windows()
            .iter()
            .filter(|w| crate::re!(r"(?i)^(?:powershell|pwsh|cmd)(?:\.exe)?$").is_match(&js::gs(w, "process")))
            .filter_map(|w| w["hwnd"].as_i64())
            .collect();
        let mut changed = false;
        {
            let alive: HashSet<i64> = wins.iter().copied().collect();
            let mut c = self.consoles.borrow_mut();
            let before = c.len();
            c.retain(|h, _| alive.contains(h));
            changed |= c.len() != before;
        }
        for h in wins {
            if !self.consoles.borrow().contains_key(&h) {
                let c = self.pick(&self.used());
                self.consoles.borrow_mut().insert(h, c);
                changed = true;
            }
        }
        changed
    }

    fn refresh_cards(&self) {
        let o = self.o.up();
        o.desktop_card_sig.borrow_mut().clear();
        o.refresh_desktop_card();
        o.terminal_voice.refresh_cards();
    }

    /// Couleurs d'une fenêtre de la card Bureau : celles de ses onglets (dans l'ordre d'ouverture), ou celle
    /// de la fenêtre console classique.
    pub fn colors_of_window(&self, hwnd: i64) -> Vec<String> {
        if let Some(c) = self.consoles.borrow().get(&hwnd) {
            return vec![hex_of(*c).to_string()];
        }
        let mut tabs: Vec<(u32, String)> = self.tabs.borrow().iter().filter(|(_, t)| t.hwnd == hwnd && hwnd != 0).map(|(p, t)| (*p, t.hex())).collect();
        tabs.sort();
        tabs.into_iter().map(|(_, c)| c).collect()
    }

    /// Onglet qui héberge ce processus (un Claude Code lancé dans l'onglet).
    fn tab_of_process(&self, pid: u32) -> Option<Tab> {
        let parents = self.parents.borrow();
        let tabs = self.tabs.borrow();
        let (mut cur, mut depth) = (pid, 0);
        while cur != 0 && depth < 16 {
            if let Some(t) = tabs.get(&cur) {
                return Some(t.clone());
            }
            cur = parents.get(&cur).copied().unwrap_or(0);
            depth += 1;
        }
        None
    }

    /// Couleur de l'onglet qui héberge ce processus.
    pub fn color_of_process(&self, pid: u32) -> Option<String> {
        self.tab_of_process(pid).map(|t| t.hex())
    }

    /// Fenêtre de Windows Terminal qui héberge ce processus.
    pub fn window_of_process(&self, pid: u32) -> Option<i64> {
        self.tab_of_process(pid).map(|t| t.hwnd).filter(|h| *h != 0)
    }
}
