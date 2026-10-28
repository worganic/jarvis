// Emplacements de fenêtres (card Bureau) : quatre dispositions mémorisées (data/emplacements.json). Une
// disposition retient, pour chaque fenêtre qu'on y a placée, sa position, sa taille et si elle était
// agrandie. Un clic sur « 2 » dans la ligne d'une fenêtre l'y envoie seule ; un clic sur « 2 » dans la barre
// de la card y envoie toutes les fenêtres de la disposition (deux terminaux, VS Code et Chrome d'un coup).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use crate::js::{self, J};

pub const SLOTS: usize = 4;

pub struct WindowSlots {
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    slots: RefCell<Vec<Vec<J>>>,
}

fn proc_key(w: &J) -> String {
    js::gs(w, "process").to_lowercase()
}

/// Entrée enregistrée d'une fenêtre ouverte (description de desktop_state).
fn entry_of(w: &J) -> J {
    json!({
        "hwnd": w["hwnd"], "process": w["process"], "app": w["app"], "title": w["title"],
        "x": w["rect"]["x"], "y": w["rect"]["y"], "w": w["rect"]["w"], "h": w["rect"]["h"],
        "max": js::gb(w, "max"), "z": w["z"]
    })
}

/// Associe les fenêtres d'une disposition aux fenêtres ouvertes : (indice d'entrée, indice de fenêtre).
/// D'abord la même fenêtre (hwnd) ; puis, pour une entrée dont la fenêtre n'existe plus (Jarvis ou
/// l'application relancés), la même application au même titre, sinon la même application, dans l'ordre.
fn assign(entries: &[J], live: &[J]) -> Vec<(usize, usize)> {
    let alive: HashSet<i64> = live.iter().filter_map(|w| w["hwnd"].as_i64()).collect();
    let mut used_e = vec![false; entries.len()];
    let mut used_w = vec![false; live.len()];
    let mut out = vec![];
    for (ei, e) in entries.iter().enumerate() {
        if let Some(wi) = live.iter().position(|w| w["hwnd"] == e["hwnd"]) {
            if !used_w[wi] {
                used_e[ei] = true;
                used_w[wi] = true;
                out.push((ei, wi));
            }
        }
    }
    for same_title in [true, false] {
        for (ei, e) in entries.iter().enumerate() {
            if used_e[ei] || e["hwnd"].as_i64().map(|h| alive.contains(&h)).unwrap_or(false) {
                continue;
            }
            let hit = live.iter().enumerate().position(|(wi, w)| !used_w[wi] && proc_key(w) == proc_key(e) && (!same_title || w["title"] == e["title"]));
            if let Some(wi) = hit {
                used_e[ei] = true;
                used_w[wi] = true;
                out.push((ei, wi));
            }
        }
    }
    out
}

impl WindowSlots {
    pub fn new(file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let mut slots = vec![vec![]; SLOTS];
        if let Some(d) = js::read_json(&file) {
            for (i, s) in slots.iter_mut().enumerate() {
                *s = js::arr(&d["slots"][(i + 1).to_string()]);
            }
        }
        Self { file, log, slots: RefCell::new(slots) }
    }

    fn save(&self) {
        let mut m = serde_json::Map::new();
        for (i, s) in self.slots.borrow().iter().enumerate() {
            m.insert((i + 1).to_string(), Value::Array(s.clone()));
        }
        if let Err(e) = js::write_json_atomic(&self.file, &json!({ "slots": m })) {
            (self.log)(format!("[emplacements] écriture impossible : {e}"));
        }
    }

    pub fn valid(n: usize) -> bool {
        (1..=SLOTS).contains(&n)
    }

    /// Nombre de fenêtres enregistrées dans chaque disposition.
    pub fn counts(&self) -> Vec<usize> {
        self.slots.borrow().iter().map(|s| s.len()).collect()
    }

    /// Pour chaque fenêtre ouverte (hwnd) : les dispositions où elle a une place.
    pub fn marks(&self, live: &[J]) -> HashMap<i64, [bool; SLOTS]> {
        let mut out: HashMap<i64, [bool; SLOTS]> = HashMap::new();
        for (i, s) in self.slots.borrow().iter().enumerate() {
            for (_, wi) in assign(s, live) {
                if let Some(h) = live[wi]["hwnd"].as_i64() {
                    out.entry(h).or_insert([false; SLOTS])[i] = true;
                }
            }
        }
        out
    }

    /// Place enregistrée de cette fenêtre dans la disposition n.
    pub fn target(&self, n: usize, live: &[J], hwnd: i64) -> Option<J> {
        let slots = self.slots.borrow();
        let s = &slots[n - 1];
        assign(s, live).into_iter().find(|(_, wi)| live[*wi]["hwnd"].as_i64() == Some(hwnd)).map(|(ei, _)| s[ei].clone())
    }

    /// Fenêtres ouvertes de la disposition n, avec leur place (de la plus en arrière à la plus en avant, pour
    /// que l'ordre des fenêtres soit celui de l'enregistrement une fois toutes ramenées). Les entrées
    /// retrouvées par leur application sont rattachées à leur nouvelle fenêtre.
    pub fn targets(&self, n: usize, live: &[J]) -> Vec<(J, J)> {
        let mut slots = self.slots.borrow_mut();
        let s = &mut slots[n - 1];
        let pairs = assign(s, live);
        let mut rebound = false;
        for (ei, wi) in &pairs {
            if s[*ei]["hwnd"] != live[*wi]["hwnd"] {
                s[*ei]["hwnd"] = live[*wi]["hwnd"].clone();
                rebound = true;
            }
        }
        let mut out: Vec<(J, J)> = pairs.iter().map(|(ei, wi)| (s[*ei].clone(), live[*wi].clone())).collect();
        drop(slots);
        if rebound {
            self.save();
        }
        out.sort_by_key(|(e, _)| std::cmp::Reverse(e["z"].as_i64().unwrap_or(0)));
        out
    }

    /// Enregistre (ou remplace) la place de cette fenêtre dans la disposition n.
    pub fn save_window(&self, n: usize, w: &J, live: &[J]) {
        {
            let mut slots = self.slots.borrow_mut();
            let s = &mut slots[n - 1];
            let hwnd = w["hwnd"].as_i64();
            let mine = assign(s, live).into_iter().find(|(_, wi)| live[*wi]["hwnd"].as_i64() == hwnd).map(|(ei, _)| ei);
            match mine {
                Some(ei) => s[ei] = entry_of(w),
                None => s.push(entry_of(w)),
            }
        }
        self.save();
    }

    /// Retire cette fenêtre de la disposition n.
    pub fn clear_window(&self, n: usize, hwnd: i64, live: &[J]) -> bool {
        let found = {
            let mut slots = self.slots.borrow_mut();
            let s = &mut slots[n - 1];
            let mine = assign(s, live).into_iter().find(|(_, wi)| live[*wi]["hwnd"].as_i64() == Some(hwnd)).map(|(ei, _)| ei);
            mine.map(|ei| s.remove(ei)).is_some()
        };
        if found {
            self.save();
        }
        found
    }

    /// Disposition n = toutes les fenêtres visibles (non réduites) telles qu'elles sont maintenant.
    pub fn save_layout(&self, n: usize, live: &[J]) -> usize {
        let list: Vec<J> = live.iter().filter(|w| !js::gb(w, "min")).map(entry_of).collect();
        let count = list.len();
        self.slots.borrow_mut()[n - 1] = list;
        self.save();
        count
    }

    pub fn clear_layout(&self, n: usize) {
        self.slots.borrow_mut()[n - 1].clear();
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(hwnd: i64, process: &str, title: &str) -> J {
        json!({ "hwnd": hwnd, "process": process, "title": title })
    }

    #[test]
    fn assign_par_fenetre_puis_par_application() {
        // Deux terminaux et VS Code enregistrés ; après relance, les hwnd ont changé.
        let entries = vec![win(1, "WindowsTerminal", "A"), win(2, "WindowsTerminal", "B"), win(3, "Code", "projet")];
        let live = vec![win(20, "WindowsTerminal", "B"), win(30, "Code", "autre"), win(10, "WindowsTerminal", "C")];
        let mut got = assign(&entries, &live);
        got.sort();
        // B retrouvé par son titre, A prend le terminal restant, VS Code par l'application.
        assert_eq!(got, vec![(0, 2), (1, 0), (2, 1)]);
    }

    #[test]
    fn une_entree_dont_la_fenetre_existe_ne_va_qu_a_elle() {
        // Le terminal 1 est ouvert : son entrée ne doit pas être prise par le terminal 2.
        let entries = vec![win(1, "WindowsTerminal", "A")];
        let live = vec![win(2, "WindowsTerminal", "A"), win(1, "WindowsTerminal", "X")];
        assert_eq!(assign(&entries, &live), vec![(0, 1)]);
        let entries = vec![win(1, "WindowsTerminal", "A")];
        let live = vec![win(1, "WindowsTerminal", "A"), win(2, "WindowsTerminal", "A")];
        assert_eq!(assign(&entries, &live), vec![(0, 0)]);
        let live2 = vec![win(2, "WindowsTerminal", "A"), win(1, "WindowsTerminal", "A")];
        assert_eq!(assign(&entries, &live2), vec![(0, 1)]);
    }
}
