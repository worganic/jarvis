// Compagnon de bureau (tamagotchi) : logique pure, sans horloge ni fichier (testée ici).
//
// Cycle de 7 ou 30 jours d'allumage (un PC éteint ne fait pas vieillir) : œuf → bébé → ado → adulte, puis
// retraite et nouvel œuf qui hérite d'un peu de la branche dominante. Quatre branches nourries par
// l'application au premier plan (une minute = 1 XP) : gamer, dev, artiste, chill. Trois jauges de 0 à 100
// (faim : 100 = rassasié, humeur, énergie) ; il ne meurt jamais, il boude.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::js::J;

pub const BRANCHES: [&str; 4] = ["gamer", "dev", "artiste", "chill"];
pub const STAGES: [&str; 4] = ["oeuf", "bebe", "ado", "adulte"];
const HISTORY_MAX: usize = 40;
const SOUVENIRS_MAX: usize = 24;
/// Part de l'XP dominante léguée au nouvel œuf.
const INHERIT: f64 = 0.10;

pub fn branch_label(b: &str) -> &'static str {
    match b {
        "gamer" => "Gamer",
        "dev" => "Bureaucrate / Dev",
        "artiste" => "Artiste",
        "chill" => "Chill / Cinéphile",
        _ => "Indécis",
    }
}

pub fn stage_label(s: &str) -> &'static str {
    match s {
        "oeuf" => "Œuf",
        "bebe" => "Bébé",
        "ado" => "Ado",
        "adulte" => "Adulte",
        _ => "Œuf",
    }
}

/// Premier jour de chaque stade après l'œuf (bébé, ado, adulte) et premier jour après le cycle.
fn stage_days(cycle: u32) -> [u32; 4] {
    if cycle <= 7 {
        [2, 3, 5, 8]
    } else {
        [3, 8, 19, 31]
    }
}

pub fn normalize_cycle(days: i64) -> u32 {
    if days <= 7 {
        7
    } else {
        30
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Xp {
    pub gamer: f64,
    pub dev: f64,
    pub artiste: f64,
    pub chill: f64,
}

impl Xp {
    pub fn get(&self, b: &str) -> f64 {
        match b {
            "gamer" => self.gamer,
            "dev" => self.dev,
            "artiste" => self.artiste,
            "chill" => self.chill,
            _ => 0.0,
        }
    }

    pub fn add(&mut self, b: &str, v: f64) {
        match b {
            "gamer" => self.gamer += v,
            "dev" => self.dev += v,
            "artiste" => self.artiste += v,
            "chill" => self.chill += v,
            _ => {}
        }
    }

    /// Branche dominante ; rien encore : chill (il regarde passer les fenêtres).
    pub fn leading(&self) -> &'static str {
        let mut best = ("chill", 0.0);
        for b in BRANCHES {
            if self.get(b) > best.1 {
                best = (b, self.get(b));
            }
        }
        best.0
    }
}

/// Ce que le service observe à chaque tic.
#[derive(Clone, Debug, Default)]
pub struct Signals {
    pub now_ms: i64,
    /// « AAAA-MM-JJ » local.
    pub day: String,
    pub hour: u32,
    /// Clavier ou souris dans les 2 dernières minutes.
    pub active: bool,
    pub idle_secs: f64,
    /// Branche de l'application au premier plan.
    pub fg_branch: Option<String>,
    pub cpu: f64,
    pub ram: f64,
    pub music: bool,
    pub game: bool,
    pub meeting: bool,
    pub offline: bool,
    pub low_battery: bool,
}

/// Ce qui vient de se passer (annoncé dans le HUD, et à la voix si l'option est cochée).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Hatched,
    Evolved(&'static str),
    Retired { generation: u32, branch: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Pet {
    pub generation: u32,
    pub born_at: i64,
    /// Jours où le PC a servi (« AAAA-MM-JJ ») : l'âge, c'est leur nombre.
    pub days: Vec<String>,
    /// Chaleur de l'œuf (0 à 100).
    pub warmth: f64,
    pub hatched: bool,
    pub stage: String,
    pub xp: Xp,
    /// Fixée au passage ado.
    pub branch: Option<String>,
    pub hunger: f64,
    pub mood: f64,
    pub energy: f64,
    /// « Ne pas déranger » : il dort jusqu'à nouvel ordre.
    pub asleep: bool,
    pub history: Vec<J>,
    pub souvenirs: Vec<J>,
}

impl Default for Pet {
    fn default() -> Self {
        Self {
            generation: 1,
            born_at: 0,
            days: vec![],
            warmth: 0.0,
            hatched: false,
            stage: "oeuf".into(),
            xp: Xp::default(),
            branch: None,
            hunger: 80.0,
            mood: 70.0,
            energy: 80.0,
            asleep: false,
            history: vec![],
            souvenirs: vec![],
        }
    }
}

fn clamp(v: f64) -> f64 {
    v.clamp(0.0, 100.0)
}

/// Repas d'un fichier déposé : seule l'extension compte (le contenu n'est jamais lu).
pub fn food_of(ext: &str) -> &'static str {
    match ext.trim_start_matches('.').to_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "heic" | "ico" => "friandise",
        "txt" | "pdf" | "doc" | "docx" | "md" | "odt" | "rtf" | "csv" | "xls" | "xlsx" | "ppt" | "pptx" | "json" | "epub" => "legumes",
        "exe" | "zip" | "rar" | "7z" | "msi" | "dll" | "iso" | "bat" | "cmd" | "tar" | "gz" => "epice",
        _ => "grignotage",
    }
}

impl Pet {
    pub fn new(now_ms: i64) -> Self {
        Self { born_at: now_ms, ..Default::default() }
    }

    pub fn age(&self) -> u32 {
        self.days.len() as u32
    }

    fn log(&mut self, now_ms: i64, kind: &str, text: String) {
        self.history.push(json!({ "at": now_ms, "kind": kind, "text": text }));
        if self.history.len() > HISTORY_MAX {
            let extra = self.history.len() - HISTORY_MAX;
            self.history.drain(..extra);
        }
    }

    /// Stade attendu pour l'âge (l'œuf reste un œuf tant qu'il n'a pas éclos).
    fn stage_for(&self, cycle: u32) -> &'static str {
        if !self.hatched {
            return "oeuf";
        }
        let [_, teen, adult, _] = stage_days(cycle);
        let age = self.age().max(1);
        if age >= adult {
            "adulte"
        } else if age >= teen {
            "ado"
        } else {
            "bebe"
        }
    }

    /// Un tic de `minutes` minutes. Rend ce qui vient de se passer.
    pub fn tick(&mut self, s: &Signals, cycle: u32, minutes: f64) -> Vec<Event> {
        let mut events = vec![];
        let m = minutes.clamp(0.0, 10.0);
        if s.active && !s.day.is_empty() && self.days.last() != Some(&s.day) && !self.days.contains(&s.day) {
            self.days.push(s.day.clone());
        }
        // Fin du cycle : il part en voyage, un nouvel œuf prend sa place.
        let [baby, _, _, end] = stage_days(cycle);
        if self.age() >= end {
            events.push(self.retire(s.now_ms));
            return events;
        }
        if s.active {
            if let Some(b) = &s.fg_branch {
                self.xp.add(b, m);
            }
        }
        if !self.hatched {
            if s.active {
                self.warmth = clamp(self.warmth + 2.5 * m);
            }
            // Assez chaud au bon jour, ou de toute façon le lendemain : jamais coincé dans l'œuf.
            if (self.warmth >= 100.0 && self.age() >= baby) || self.age() > baby {
                self.hatched = true;
                self.log(s.now_ms, "eclosion", "L'œuf a éclos !".into());
                events.push(Event::Hatched);
            }
        }
        // Jauges.
        self.hunger = clamp(self.hunger - if s.active { 0.35 } else { 0.1 } * m);
        let mut mood = if self.mood > 55.0 { -0.04 } else { 0.03 };
        if s.music {
            mood += 0.15;
        }
        if s.idle_secs > 1800.0 {
            mood -= 0.05; // longue solitude
        }
        if self.hunger < 20.0 {
            mood -= 0.1;
        }
        self.mood = clamp(self.mood + mood * m);
        let mut energy = if s.active { -0.12 } else { 0.5 };
        if s.active && (1..5).contains(&s.hour) {
            energy -= 0.2;
        }
        if self.asleep {
            energy = 0.6;
        }
        self.energy = clamp(self.energy + energy * m);
        // Évolution.
        let want = self.stage_for(cycle);
        if want != self.stage {
            if want == "ado" && self.branch.is_none() {
                self.branch = Some(self.xp.leading().to_string());
            }
            self.stage = want.to_string();
            if want != "bebe" || !events.contains(&Event::Hatched) {
                let b = self.branch.as_deref().map(branch_label).unwrap_or("");
                let text = if b.is_empty() { format!("Il devient {}.", stage_label(want)) } else { format!("Il devient {} {b}.", stage_label(want)) };
                self.log(s.now_ms, "evolution", text);
                events.push(Event::Evolved(want));
            }
        }
        events
    }

    fn retire(&mut self, now_ms: i64) -> Event {
        let branch = self.branch.clone().unwrap_or_else(|| self.xp.leading().to_string());
        let generation = self.generation;
        let mut souvenirs = std::mem::take(&mut self.souvenirs);
        souvenirs.push(json!({
            "generation": generation, "branch": branch, "stage": self.stage, "days": self.age(),
            "born_at": self.born_at, "retired_at": now_ms, "xp": self.xp
        }));
        if souvenirs.len() > SOUVENIRS_MAX {
            let extra = souvenirs.len() - SOUVENIRS_MAX;
            souvenirs.drain(..extra);
        }
        let inherited = self.xp.get(&branch) * INHERIT;
        let mut next = Pet::new(now_ms);
        next.generation = generation + 1;
        next.xp.add(&branch, inherited);
        next.souvenirs = souvenirs;
        next.log(now_ms, "retraite", format!("La génération {generation} ({}) part en voyage et laisse un nouvel œuf.", branch_label(&branch)));
        *self = next;
        Event::Retired { generation, branch }
    }

    /// Nouvel œuf demandé (admin, card, voix) : même chose qu'une fin de cycle.
    pub fn restart(&mut self, now_ms: i64) -> Event {
        self.retire(now_ms)
    }

    /// Fichier déposé : (repas, accepté). Rassasié, il refuse poliment.
    pub fn feed(&mut self, ext: &str, now_ms: i64) -> (&'static str, bool) {
        let food = food_of(ext);
        if self.hunger >= 98.0 {
            return (food, false);
        }
        // Boudeur : il mange moins vite.
        let k = if self.mood < 25.0 { 0.5 } else { 1.0 };
        match food {
            "friandise" => {
                self.hunger = clamp(self.hunger + 8.0 * k);
                self.mood = clamp(self.mood + 6.0);
            }
            "legumes" => {
                self.hunger = clamp(self.hunger + 12.0 * k);
                self.xp.add("dev", 5.0);
            }
            "epice" => {
                self.hunger = clamp(self.hunger + 10.0 * k);
                self.mood = clamp(self.mood + 2.0);
            }
            _ => self.hunger = clamp(self.hunger + 5.0 * k),
        }
        if !self.hatched {
            self.warmth = clamp(self.warmth + 5.0);
        }
        let what = match food {
            "friandise" => "une friandise (image)",
            "legumes" => "des légumes (document)",
            "epice" => "un plat épicé (programme ou archive)",
            _ => "un petit en-cas",
        };
        self.log(now_ms, "repas", format!("Il a mangé {what}."));
        (food, true)
    }

    /// Corbeille vidée : pluie de croquettes de données.
    pub fn kibble(&mut self, now_ms: i64) {
        self.hunger = clamp(self.hunger + 15.0);
        self.mood = clamp(self.mood + 5.0);
        self.log(now_ms, "repas", "Corbeille vidée : pluie de croquettes !".into());
    }

    pub fn caress(&mut self) {
        self.mood = clamp(self.mood + 3.0);
    }

    /// Demande traitée par Jarvis : il s'en réjouit (ou s'en attriste un peu) ; l'œuf se réchauffe.
    pub fn request_done(&mut self, ok: bool) {
        self.mood = clamp(self.mood + if ok { 1.5 } else { -2.0 });
        if !self.hatched {
            self.warmth = clamp(self.warmth + 3.0);
        }
    }

    /// Effets visibles en ce moment (le widget et le personnage les dessinent).
    pub fn effects(&self, s: &Signals) -> Vec<&'static str> {
        let mut e = vec![];
        if self.asleep {
            e.push("asleep");
        }
        if s.game {
            e.push("stealth");
        }
        if s.meeting {
            e.push("meeting");
        }
        if s.cpu > 80.0 || s.ram > 90.0 {
            e.push("hot");
        }
        if s.active && (1..5).contains(&s.hour) {
            e.push("night");
        }
        if s.idle_secs > 600.0 && !self.asleep {
            e.push("afk");
        }
        if s.offline {
            e.push("offline");
        }
        if s.low_battery {
            e.push("lowbattery");
        }
        if s.music {
            e.push("music");
        }
        if self.hunger < 25.0 {
            e.push("hungry");
        }
        if self.mood < 25.0 {
            e.push("sulky");
        }
        if self.energy < 20.0 {
            e.push("tired");
        }
        e
    }

    /// Accessoires dessinés selon le stade et la branche.
    pub fn accessories(&self) -> Vec<&'static str> {
        let branch = self.branch.as_deref().unwrap_or("");
        let set: &[&'static str] = match branch {
            "gamer" => &["casque", "rgb"],
            "dev" => &["lunettes", "tasse"],
            "artiste" => &["beret", "peinture"],
            "chill" => &["plaid", "popcorn"],
            _ => &[],
        };
        match self.stage.as_str() {
            "ado" => set.iter().take(1).copied().collect(),
            "adulte" => {
                let mut v: Vec<&'static str> = set.to_vec();
                v.push("aura");
                v
            }
            _ => vec![],
        }
    }

    /// Ce que voient le widget, la card et la page d'aide.
    pub fn view(&self, s: &Signals, cycle: u32) -> J {
        let [_, _, _, end] = stage_days(cycle);
        json!({
            "generation": self.generation, "stage": self.stage, "stageLabel": stage_label(&self.stage),
            "day": self.age().max(1), "cycle": cycle, "daysLeft": end.saturating_sub(self.age().max(1)),
            "hatched": self.hatched, "warmth": self.warmth.round(),
            "branch": self.branch, "branchLabel": self.branch.as_deref().map(branch_label),
            "leading": self.xp.leading(), "xp": self.xp,
            "hunger": self.hunger.round(), "mood": self.mood.round(), "energy": self.energy.round(),
            "asleep": self.asleep, "effects": self.effects(s), "accessories": self.accessories(),
            "history": self.history.iter().rev().take(20).cloned().collect::<Vec<_>>(),
            "souvenirs": self.souvenirs.iter().rev().cloned().collect::<Vec<_>>()
        })
    }

    /// Phrase courte de l'état (« comment va mon compagnon ? »).
    pub fn describe(&self, cycle: u32) -> String {
        let stage = stage_label(&self.stage);
        let branch = self.branch.as_deref().map(|b| format!(" {}", branch_label(b))).unwrap_or_default();
        let feeling = if self.asleep {
            "Il dort, en mode ne pas déranger."
        } else if self.hunger < 25.0 {
            "Il a faim : glisse-lui un fichier."
        } else if self.mood < 25.0 {
            "Il boude un peu."
        } else if self.energy < 20.0 {
            "Il est fatigué."
        } else if self.mood > 75.0 {
            "Il est de très bonne humeur."
        } else {
            "Il va bien."
        };
        if !self.hatched {
            return format!("C'est encore un œuf, chaud à {} %. {feeling}", self.warmth.round());
        }
        format!("{stage}{branch}, jour {} sur {cycle}. {feeling}", self.age().max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(day: &str, branch: Option<&str>) -> Signals {
        Signals { day: day.into(), hour: 14, active: true, fg_branch: branch.map(String::from), ..Default::default() }
    }

    fn live_days(p: &mut Pet, cycle: u32, from: u32, to: u32, branch: Option<&str>) -> Vec<Event> {
        let mut all = vec![];
        for d in from..=to {
            let s = sig(&format!("2026-01-{d:02}"), branch);
            for _ in 0..60 {
                all.extend(p.tick(&s, cycle, 5.0));
            }
        }
        all
    }

    #[test]
    fn stades_sur_trente_jours() {
        let mut p = Pet::new(0);
        live_days(&mut p, 30, 1, 2, None);
        assert_eq!(p.stage, "oeuf", "œuf les jours 1 et 2");
        live_days(&mut p, 30, 3, 3, None);
        assert_eq!(p.stage, "bebe");
        live_days(&mut p, 30, 4, 8, Some("dev"));
        assert_eq!(p.stage, "ado");
        live_days(&mut p, 30, 9, 19, None);
        assert_eq!(p.stage, "adulte");
    }

    #[test]
    fn stades_sur_sept_jours() {
        let mut p = Pet::new(0);
        live_days(&mut p, 7, 1, 1, None);
        assert_eq!(p.stage, "oeuf");
        live_days(&mut p, 7, 2, 2, None);
        assert_eq!(p.stage, "bebe");
        live_days(&mut p, 7, 3, 3, None);
        assert_eq!(p.stage, "ado");
        live_days(&mut p, 7, 4, 5, None);
        assert_eq!(p.stage, "adulte");
    }

    #[test]
    fn un_pc_eteint_ne_fait_pas_vieillir() {
        let mut p = Pet::new(0);
        let mut s = sig("2026-01-01", None);
        p.tick(&s, 30, 5.0);
        s.active = false;
        s.day = "2026-01-05".into();
        p.tick(&s, 30, 5.0);
        assert_eq!(p.age(), 1);
    }

    #[test]
    fn la_branche_se_fixe_a_l_adolescence() {
        let mut p = Pet::new(0);
        live_days(&mut p, 30, 1, 5, Some("gamer"));
        live_days(&mut p, 30, 6, 8, Some("artiste"));
        assert_eq!(p.stage, "ado");
        assert_eq!(p.branch.as_deref(), Some("gamer"));
        assert_eq!(p.accessories(), vec!["casque"]);
        // L'adulte garde sa branche, même si l'usage change ensuite.
        live_days(&mut p, 30, 9, 20, Some("artiste"));
        assert_eq!(p.branch.as_deref(), Some("gamer"));
        assert!(p.accessories().contains(&"aura"));
    }

    #[test]
    fn jauges_bornees_et_repas() {
        let mut p = Pet::new(0);
        p.hunger = 10.0;
        assert_eq!(p.feed("PNG", 0), ("friandise", true));
        assert_eq!(p.feed(".zip", 0).0, "epice");
        let dev = p.xp.dev;
        p.feed("pdf", 0);
        assert!(p.xp.dev > dev);
        p.hunger = 99.0;
        assert_eq!(p.feed("txt", 0), ("legumes", false), "rassasié, il refuse");
        for _ in 0..50 {
            p.kibble(0);
            p.caress();
        }
        assert!(p.hunger <= 100.0 && p.mood <= 100.0);
        let s = Signals { active: false, ..Default::default() };
        for _ in 0..1000 {
            p.tick(&s, 30, 10.0);
        }
        assert!(p.hunger >= 0.0 && p.energy <= 100.0);
    }

    #[test]
    fn fin_de_cycle_et_heritage() {
        let mut p = Pet::new(0);
        let events = live_days(&mut p, 7, 1, 8, Some("chill"));
        assert!(events.iter().any(|e| matches!(e, Event::Retired { generation: 1, .. })));
        assert_eq!(p.generation, 2);
        assert_eq!(p.stage, "oeuf");
        assert_eq!(p.souvenirs.len(), 1);
        assert!(p.xp.chill > 0.0, "le nouvel œuf hérite de la branche dominante");
    }
}
