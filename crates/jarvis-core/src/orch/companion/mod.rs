// Compagnon de bureau (tamagotchi) : le cœur choisi vit, grandit et évolue selon l'usage du PC (option de
// l'administration, désactivée par défaut). Logique pure dans model.rs.
//
// Jarvis reste prioritaire : le compagnon ne parle jamais de lui-même (sauf, si l'option est cochée, une
// phrase à une évolution, par la file de parole normale), ne touche ni à la voix ni aux commandes. Le widget et
// le personnage reçoivent l'événement « companion » et ne l'animent qu'au repos (ui/character-director.js).
// Rien n'est lu des fichiers déposés ni de l'écran : nom du processus au premier plan, temps d'inactivité,
// charge CPU/RAM, batterie, nombre d'éléments de la corbeille. État : data/companion.json.

pub mod model;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use self::model::{Event, Pet, Signals};
use super::intent::Intent;
use super::skill_intents::it;
use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

const TICK_MS: u64 = 30_000;
/// Clavier ou souris dans ce délai : l'utilisateur est là.
const ACTIVE_S: f64 = 120.0;
const NET_CHECK_MS: i64 = 120_000;
const CARESS_MS: i64 = 20_000;
const FEED_MS: i64 = 3_000;

static BRANCHES: LazyLock<Vec<(regex::Regex, String)>> = LazyLock::new(|| {
    let raw: J = serde_json::from_str(include_str!("../../../data/companion-branches.json")).unwrap_or_default();
    js::arr(&raw["branches"])
        .iter()
        .filter_map(|b| Some((regex::Regex::new(&format!("(?i){}", js::gs(b, "pattern"))).ok()?, js::gs(b, "branch"))))
        .collect()
});

/// Branche de l'application au premier plan (nom du processus, avec ou sans « .exe »).
pub fn branch_of_process(process: &str) -> Option<String> {
    let p = process.trim().trim_end_matches(".exe").trim_end_matches(".EXE");
    if p.is_empty() || p.to_lowercase().starts_with("jarvis") {
        return None;
    }
    BRANCHES.iter().find(|(re, _)| re.is_match(p)).map(|(_, b)| b.clone())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub enabled: bool,
    pub cycle_days: u32,
    /// Une phrase à voix haute aux évolutions (sinon écrite seulement).
    pub voice_announce: bool,
    /// Mode furtif en jeu et en plein écran.
    pub auto_stealth: bool,
    /// Promenade le long de la barre des tâches au repos.
    pub walk: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: false, cycle_days: 30, voice_announce: false, auto_stealth: true, walk: true }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    config: Config,
    pet: Option<Pet>,
}

pub struct Companion {
    o: OrchRef,
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    stored: RefCell<Stored>,
    timer: Timer,
    fg_branch: RefCell<Option<String>>,
    signals: RefCell<Signals>,
    last_tick: Cell<i64>,
    recycle: Cell<Option<i64>>,
    offline: Cell<bool>,
    net_at: Cell<i64>,
    caress_at: Cell<i64>,
    feed_at: Cell<i64>,
    last_view: RefCell<String>,
    /// Effet passager (repas, croquettes, caresse) : (nom, jusqu'à).
    flash: RefCell<Option<(String, i64)>>,
}

impl Companion {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let stored: Stored = js::read_json(&file).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        Self {
            o,
            file,
            log,
            stored: RefCell::new(stored),
            timer: Timer::new(),
            fg_branch: RefCell::new(None),
            signals: RefCell::new(Signals::default()),
            last_tick: Cell::new(0),
            recycle: Cell::new(None),
            offline: Cell::new(false),
            net_at: Cell::new(0),
            caress_at: Cell::new(0),
            feed_at: Cell::new(0),
            last_view: RefCell::new(String::new()),
            flash: RefCell::new(None),
        }
    }

    pub fn config(&self) -> Config {
        self.stored.borrow().config.clone()
    }

    pub fn enabled(&self) -> bool {
        self.stored.borrow().config.enabled
    }

    fn save(&self) {
        let v = serde_json::to_value(&*self.stored.borrow()).unwrap_or(Value::Null);
        let _ = js::write_json(&self.file, &v);
    }

    fn cycle(&self) -> u32 {
        model::normalize_cycle(self.stored.borrow().config.cycle_days as i64)
    }

    /// Démarrage : suivi du premier plan, puis un tic toutes les 30 s (rien ne tourne si l'option est coupée).
    pub fn start(&self) {
        let o = self.o.clone();
        self.o.up().desktop_state.on("foreground", move |w| {
            let me = &o.up().companion;
            *me.fg_branch.borrow_mut() = branch_of_process(&js::gs(w, "process"));
        });
        let o = self.o.clone();
        self.timer.every(TICK_MS, move || {
            let o = o.clone();
            js::spawn(async move {
                o.up().companion.tick().await;
            });
        });
        let o = self.o.clone();
        js::set_timeout(4000, move || o.up().companion.publish(true));
    }

    async fn read_signals(&self) -> Signals {
        let o = self.o.up();
        let now = js::now_ms();
        let local = js::local_of(now);
        let (idle, battery) = tokio::task::spawn_blocking(|| (jarvis_native::presence::idle_seconds(), jarvis_native::presence::battery())).await.unwrap_or((0.0, None));
        let tele = super::system::telemetry();
        let cm = o.context_mute.get_state();
        let category = js::gs(&cm["activeApp"], "category");
        if now - self.net_at.get() > NET_CHECK_MS {
            self.net_at.set(now);
            let ok = tokio::time::timeout(std::time::Duration::from_secs(3), tokio::net::TcpStream::connect("1.1.1.1:443")).await.map(|r| r.is_ok()).unwrap_or(false);
            self.offline.set(!ok);
        }
        let auto_stealth = self.stored.borrow().config.auto_stealth;
        Signals {
            now_ms: now,
            day: local.format("%Y-%m-%d").to_string(),
            hour: local.format("%H").to_string().parse().unwrap_or(12),
            active: idle < ACTIVE_S,
            idle_secs: idle,
            fg_branch: self.fg_branch.borrow().clone(),
            cpu: js::gn_or(&tele, "cpu", 0.0),
            ram: js::gn_or(&tele, "ramPct", 0.0),
            music: category == "musique",
            game: auto_stealth && category == "jeu",
            meeting: category == "visio",
            offline: self.offline.get(),
            low_battery: battery.map(|(on_battery, pct)| on_battery && pct <= 20).unwrap_or(false),
        }
    }

    async fn tick(&self) {
        if !self.enabled() {
            return;
        }
        let s = self.read_signals().await;
        // Corbeille vidée depuis le dernier tic : pluie de croquettes.
        let bin = tokio::task::spawn_blocking(jarvis_native::presence::recycle_bin_count).await.ok().flatten();
        let emptied = matches!((self.recycle.get(), bin), (Some(before), Some(0)) if before > 0);
        self.recycle.set(bin);
        let now = s.now_ms;
        let minutes = if self.last_tick.get() == 0 { 0.5 } else { ((now - self.last_tick.get()) as f64 / 60000.0).clamp(0.0, 10.0) };
        self.last_tick.set(now);
        let cycle = self.cycle();
        let events = {
            let mut st = self.stored.borrow_mut();
            let pet = st.pet.get_or_insert_with(|| Pet::new(now));
            if emptied {
                pet.kibble(now);
            }
            pet.tick(&s, cycle, minutes)
        };
        if emptied {
            self.flash("croquettes", 6000);
        }
        *self.signals.borrow_mut() = s;
        self.save();
        for e in events {
            self.announce(&e);
        }
        self.publish(false);
    }

    fn flash(&self, name: &str, ms: i64) {
        *self.flash.borrow_mut() = Some((name.to_string(), js::now_ms() + ms));
    }

    /// Évolution, éclosion, retraite : écrit dans le journal du HUD, dit seulement si l'option est cochée.
    fn announce(&self, e: &Event) {
        let o = self.o.up();
        let text = match e {
            Event::Hatched => "Ton compagnon vient d'éclore !".to_string(),
            Event::Evolved(stage) => {
                let st = self.stored.borrow();
                let branch = st.pet.as_ref().and_then(|p| p.branch.clone()).map(|b| format!(" {}", model::branch_label(&b))).unwrap_or_default();
                format!("Ton compagnon devient {}{branch} !", model::stage_label(stage))
            }
            Event::Retired { generation, branch } => format!("Ton compagnon de génération {generation} ({}) part en voyage et te laisse un nouvel œuf.", model::branch_label(branch)),
        };
        (self.log)(format!("compagnon : {text}"));
        o.record("compagnon", json!({ "evenement": format!("{e:?}"), "texte": text }));
        let kind = match e {
            Event::Hatched => "hatch",
            Event::Evolved(_) => "evolve",
            Event::Retired { .. } => "retire",
        };
        self.flash(kind, 8000);
        if self.stored.borrow().config.voice_announce {
            o.emit(json!({ "type": "say", "text": text, "companion": true }));
        } else {
            o.emit(json!({ "type": "companion_note", "text": text }));
        }
    }

    /// État complet pour le widget, le personnage, la card et l'administration.
    pub fn view(&self) -> J {
        let st = self.stored.borrow();
        let cycle = self.cycle();
        let signals = self.signals.borrow();
        let mut v = match &st.pet {
            Some(p) => p.view(&signals, cycle),
            None => Pet::new(js::now_ms()).view(&signals, cycle),
        };
        v["enabled"] = json!(st.config.enabled);
        v["config"] = serde_json::to_value(&st.config).unwrap_or(Value::Null);
        let now = js::now_ms();
        let flash = self.flash.borrow().clone().filter(|(_, until)| *until > now);
        v["flash"] = json!(flash.map(|(n, _)| n));
        v["describe"] = json!(st.pet.as_ref().map(|p| p.describe(cycle)).unwrap_or_else(|| "C'est un œuf tout neuf.".into()));
        v
    }

    /// Diffuse l'état (au plus quand il change, sauf `force`).
    pub fn publish(&self, force: bool) {
        let mut v = self.view();
        let key = sha_key(&v);
        if !force && *self.last_view.borrow() == key {
            return;
        }
        *self.last_view.borrow_mut() = key;
        v["type"] = json!("companion");
        self.o.up().emit(v);
    }

    // --- Actions (card, widget, administration, voix) ------------------------------------------

    pub fn set_config(&self, patch: &J) -> J {
        {
            let mut st = self.stored.borrow_mut();
            let c = &mut st.config;
            if let Some(b) = patch["enabled"].as_bool() {
                c.enabled = b;
            }
            if let Some(n) = patch["cycleDays"].as_i64() {
                c.cycle_days = model::normalize_cycle(n);
            }
            for (k, slot) in [("voiceAnnounce", &mut c.voice_announce), ("autoStealth", &mut c.auto_stealth), ("walk", &mut c.walk)] {
                if let Some(b) = patch[k].as_bool() {
                    *slot = b;
                }
            }
            if c.enabled && st.pet.is_none() {
                st.pet = Some(Pet::new(js::now_ms()));
            }
        }
        self.save();
        self.o.up().record("compagnon_reglage", serde_json::to_value(self.config()).unwrap_or(Value::Null));
        self.publish(true);
        json!({ "ok": true, "state": self.view() })
    }

    pub fn set_asleep(&self, on: bool) -> J {
        if let Some(p) = self.stored.borrow_mut().pet.as_mut() {
            p.asleep = on;
        }
        self.save();
        self.publish(true);
        json!({ "ok": true, "asleep": on })
    }

    pub fn restart(&self) -> J {
        let e = {
            let mut st = self.stored.borrow_mut();
            let now = js::now_ms();
            st.pet.get_or_insert_with(|| Pet::new(now)).restart(now)
        };
        self.save();
        self.announce(&e);
        self.publish(true);
        json!({ "ok": true, "state": self.view() })
    }

    /// Fichier déposé sur le widget : seule son extension arrive ici (jamais le chemin ni le contenu).
    pub fn feed(&self, ext: &str) -> J {
        if !self.enabled() {
            return json!({ "ok": false, "error": "Compagnon désactivé." });
        }
        let now = js::now_ms();
        if now - self.feed_at.get() < FEED_MS {
            return json!({ "ok": false, "error": "Il mâche encore." });
        }
        self.feed_at.set(now);
        let ext: String = ext.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
        let (food, eaten) = self.stored.borrow_mut().pet.get_or_insert_with(|| Pet::new(now)).feed(&ext, now);
        if eaten {
            self.flash(food, if food == "epice" { 5000 } else { 4000 });
        } else {
            self.flash("rassasie", 2500);
        }
        self.save();
        self.publish(true);
        json!({ "ok": true, "food": food, "eaten": eaten })
    }

    pub fn caress(&self) -> J {
        let now = js::now_ms();
        if !self.enabled() || now - self.caress_at.get() < CARESS_MS {
            return json!({ "ok": true, "purr": true });
        }
        self.caress_at.set(now);
        if let Some(p) = self.stored.borrow_mut().pet.as_mut() {
            p.caress();
        }
        self.flash("ronron", 3000);
        self.save();
        self.publish(true);
        json!({ "ok": true, "purr": true })
    }

    /// Demande traitée par Jarvis (événement « finish ») : un peu d'humeur, et l'œuf se réchauffe.
    pub fn on_task_finished(&self, ok: bool) {
        if !self.enabled() {
            return;
        }
        if let Some(p) = self.stored.borrow_mut().pet.as_mut() {
            p.request_done(ok);
        }
    }

    // --- Voix ----------------------------------------------------------------------------------

    /// « comment va mon compagnon », « mets le compagnon en veille », « réveille le compagnon », « nouvel œuf ».
    /// Phrases entières seulement : « compagnon » désigne aussi un conjoint (« écris à mon compagnon » n'est pas pour lui).
    pub fn intent(o: &OrchRef, t: &str) -> Option<Intent> {
        let action = parse_command(t)?;
        let (sleep, wake, on, off, restart, open) = (action == "veille", action == "reveil", action == "activer", action == "desactiver", action == "oeuf", action == "ouvrir");
        if sleep {
            return Some(it(o, "companion_sleep", "Compagnon", "en veille", |o| async move {
                o.companion.set_asleep(true);
                json!({ "say": "Ton compagnon dort jusqu'à nouvel ordre.", "display": "Compagnon en veille" })
            }));
        }
        if wake {
            return Some(it(o, "companion_wake", "Compagnon", "réveil", |o| async move {
                o.companion.set_asleep(false);
                json!({ "say": "Ton compagnon est réveillé.", "display": "Compagnon réveillé" })
            }));
        }
        if on {
            return Some(it(o, "companion_on", "Compagnon", "activé", |o| async move {
                o.companion.set_config(&json!({ "enabled": true }));
                json!({ "say": "Compagnon activé : un œuf vient d'apparaître.", "display": "Compagnon activé" })
            }));
        }
        if off {
            return Some(it(o, "companion_off", "Compagnon", "désactivé", |o| async move {
                o.companion.set_config(&json!({ "enabled": false }));
                json!({ "say": "Compagnon désactivé : le cœur reprend son apparence habituelle.", "display": "Compagnon désactivé" })
            }));
        }
        if restart {
            return Some(it(o, "companion_restart", "Compagnon", "nouvel œuf", |o| async move {
                if !o.companion.enabled() {
                    return json!({ "say": "Le compagnon est désactivé. Dis « active le compagnon ».", "display": "Compagnon désactivé" });
                }
                o.companion.restart();
                json!({ "say": "C'est fait : ton compagnon part en voyage et un nouvel œuf l'a remplacé.", "display": "Nouvel œuf" })
            }));
        }
        if open {
            return Some(it(o, "companion_card", "Compagnon", "card", |o| async move {
                o.emit(json!({ "type": "companion_open" }));
                json!({ "say": "", "display": "Card Compagnon" })
            }));
        }
        Some(it(o, "companion_state", "Compagnon", "état", |o| async move {
            if !o.companion.enabled() {
                return json!({ "say": "Le compagnon est désactivé. Tu peux l'activer dans l'administration, ou dire « active le compagnon ».", "display": "Compagnon désactivé" });
            }
            let d = js::gs(&o.companion.view(), "describe");
            json!({ "say": d, "display": d })
        }))
    }
}

/// Commande vocale du compagnon dans une phrase repliée (minuscules, sans accents) : etat, veille, reveil,
/// activer, desactiver, oeuf, ouvrir. Phrases entières seulement : « compagnon » désigne aussi un conjoint.
pub fn parse_command(t: &str) -> Option<&'static str> {
    let t = crate::re!(r"^(?:jarvis\s+)?").replace(t.trim(), "").to_string();
    let pet = r"(?:le |mon |ton |du |au |la card )?(?:compagnon(?: de bureau)?|tamagotchi|tamagoshi)";
    let whole = |verbs: &str| regex::Regex::new(&format!(r"^(?:{verbs}) {pet}(?: .{{0,20}})?$")).map(|r| r.is_match(&t)).unwrap_or(false);
    if whole(r"mets|met|endors|fais dormir|couche") && crate::re!(r"(?:veille|dodo|dormir|endors|couche|ne pas deranger)").is_match(&t) {
        return Some("veille");
    }
    if crate::re!(r"^(?:un )?nouvel oeuf$|^fais (?:naitre|eclore) un nouvel oeuf$").is_match(&t) || whole(r"recommence|reinitialise|redemarre") {
        return Some("oeuf");
    }
    let table: [(&str, &'static str); 5] = [
        (r"reveille|debout", "reveil"),
        (r"desactive|eteins|coupe|enleve", "desactiver"),
        (r"active|allume|lance", "activer"),
        (r"ouvre|affiche|montre|montre moi|affiche moi", "ouvrir"),
        (r"comment va|comment se porte|ou en est|quel age a|etat du|l etat du|donne moi l etat du|des nouvelles du", "etat"),
    ];
    table.iter().find(|(verbs, _)| whole(verbs)).map(|(_, a)| *a)
}

fn sha_key(v: &J) -> String {
    let mut v = v.clone();
    // L'historique et les souvenirs ne changent pas l'animation : ils n'imposent pas une diffusion.
    if let Some(m) = v.as_object_mut() {
        m.remove("history");
        m.remove("souvenirs");
    }
    super::nomade::sources::hash(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processus_vers_branche() {
        assert_eq!(branch_of_process("Code.exe").as_deref(), Some("dev"));
        assert_eq!(branch_of_process("steam").as_deref(), Some("gamer"));
        assert_eq!(branch_of_process("Photoshop.exe").as_deref(), Some("artiste"));
        assert_eq!(branch_of_process("Spotify.exe").as_deref(), Some("chill"));
        assert_eq!(branch_of_process("jarvis-shell.exe"), None, "Jarvis ne compte pas");
        assert_eq!(branch_of_process("explorer.exe"), None);
    }

    #[test]
    fn commandes_vocales_sans_detourner_le_conjoint() {
        assert_eq!(parse_command("comment va mon compagnon"), Some("etat"));
        assert_eq!(parse_command("mets le compagnon en veille"), Some("veille"));
        assert_eq!(parse_command("reveille le compagnon"), Some("reveil"));
        assert_eq!(parse_command("active le compagnon"), Some("activer"));
        assert_eq!(parse_command("desactive le tamagotchi"), Some("desactiver"));
        assert_eq!(parse_command("nouvel oeuf"), Some("oeuf"));
        assert_eq!(parse_command("ouvre la card compagnon"), Some("ouvrir"));
        assert_eq!(parse_command("envoie un message a mon compagnon"), None);
        assert_eq!(parse_command("appelle mon compagnon"), None);
        assert_eq!(parse_command("mon compagnon rentre a quelle heure"), None);
    }

    #[test]
    fn la_table_des_branches_est_valide() {
        assert_eq!(BRANCHES.len(), 4, "les 4 motifs de data/companion-branches.json se compilent");
        assert!(BRANCHES.iter().all(|(_, b)| model::BRANCHES.contains(&b.as_str())));
    }
}
