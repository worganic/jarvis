// Mises à jour d'un Jarvis installé (installer.ps1) : les versions sont les tags v4.x.y du dépôt public de diffusion
// (install.json › depot), publiés par tools/publier.ps1 ; « -beta » n'est vu que sur le canal bêta.
//
// Vérification peu après le démarrage puis toutes les 6 h (API GitHub, sans jeton). Réglage « update.mode » :
//   auto      la nouvelle version est préparée seule (récupérée et compilée en arrière-plan, Jarvis reste ouvert),
//             puis installée à la fermeture de Jarvis (la coquille lance « installer.ps1 -Appliquer -SansRelancer ») ;
//   demander  (défaut) Jarvis annonce la version et ses nouveautés ; « Préparer » puis « Installer et redémarrer » ;
//   manuel    aucune vérification automatique (bouton « Vérifier » de l'administration).
// En développement (lancé depuis les sources), rien de tout cela : la recompilation locale reste la règle (relaunch.rs).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use serde_json::{json, Value as J};

use crate::js::{self, Timer};

const EVERY_MS: u64 = 6 * 3600 * 1000;
const FIRST_MS: u64 = 60_000;

/// Version « 4.101.0 » ou tag « v4.101.0-beta » → (4, 101, 0, bêta).
pub fn parse(v: &str) -> Option<(u64, u64, u64, bool)> {
    let v = v.trim().trim_start_matches('v');
    let (num, beta) = match v.strip_suffix("-beta") {
        Some(n) => (n, true),
        None => (v, false),
    };
    let mut p = num.split('.').map(|x| x.parse::<u64>().ok());
    Some((p.next()??, p.next()??, p.next()??, beta))
}

/// Tag le plus récent du canal, s'il est plus récent que `current`. À numéro égal, la version stable passe
/// avant la bêta (une bêta n'est jamais proposée après sa version stable).
pub fn newest(tags: &[String], current: &str, beta: bool) -> Option<String> {
    let cur = parse(current)?;
    tags.iter()
        .filter_map(|t| parse(t).map(|p| (p, t)))
        .filter(|(p, _)| beta || !p.3)
        .filter(|(p, _)| (p.0, p.1, p.2) > (cur.0, cur.1, cur.2) || ((p.0, p.1, p.2) == (cur.0, cur.1, cur.2) && cur.3 && !p.3))
        .max_by_key(|(p, _)| (p.0, p.1, p.2, !p.3))
        .map(|(_, t)| t.clone())
}

fn build_dir() -> Option<PathBuf> {
    crate::paths::base_dir().map(|b| b.join("build"))
}

/// Version compilée en attente d'installation (installer.ps1 -Preparer).
pub fn ready() -> Option<J> {
    build_dir().and_then(|d| js::read_json(&d.join("pret.json")))
}

pub struct Updater {
    config: Box<dyn Fn() -> J>,
    emit: Rc<dyn Fn(J)>,
    log: Rc<dyn Fn(String)>,
    timer: Timer,
    state: RefCell<J>,
    preparing: Cell<bool>,
    me: RefCell<Weak<Updater>>,
}

impl Updater {
    pub fn new(config: Box<dyn Fn() -> J>, emit: Rc<dyn Fn(J)>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let u = Rc::new(Self { config, emit, log, timer: Timer::new(), state: RefCell::new(json!({})), preparing: Cell::new(false), me: RefCell::new(Weak::new()) });
        *u.me.borrow_mut() = Rc::downgrade(&u);
        u.refresh_local();
        u
    }

    fn mode(&self) -> String {
        Some(js::gs(&(self.config)(), "mode")).filter(|m| ["auto", "demander", "manuel"].contains(&m.as_str())).unwrap_or_else(|| "demander".into())
    }

    /// État affiché (HUD, administration) : version installée, disponible, prête, préparation en cours.
    pub fn state(&self) -> J {
        self.state.borrow().clone()
    }

    fn set(&self, patch: J) {
        js::assign(&mut self.state.borrow_mut(), patch);
        let mut ev = json!({ "type": "update_state" });
        js::assign(&mut ev, self.state());
        (self.emit)(ev);
    }

    fn refresh_local(&self) {
        let info = crate::paths::installed();
        let r = ready();
        js::assign(
            &mut self.state.borrow_mut(),
            json!({
                "installe": info.is_some(),
                "actuelle": info.as_ref().map(|i| js::gs(i, "version")).unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
                "canal": info.as_ref().map(|i| js::gs(i, "canal")).filter(|c| !c.is_empty()).unwrap_or_else(|| "stable".into()),
                "depot": info.as_ref().map(|i| js::gs(i, "depot")).unwrap_or_default(),
                "pret": r.as_ref().map(|r| r["tag"].clone()).unwrap_or(J::Null),
                "preparation": self.preparing.get(),
                "mode": self.mode(),
                // Installation à la carte : modules installés et catalogue de la version installée.
                "modules": info.as_ref().map(|i| i["modules"].clone()).unwrap_or(J::Null),
                "catalogue": js::read_json(&crate::paths::root().join("modules").join("catalogue.json")).map(|c| c["modules"].clone()).unwrap_or(J::Null),
            }),
        );
    }

    pub fn start(self: &Rc<Self>) {
        if crate::paths::installed().is_none() {
            return;
        }
        let me = Rc::downgrade(self);
        js::set_timeout(FIRST_MS, {
            let me = me.clone();
            move || {
                if let Some(u) = me.upgrade() {
                    u.tick();
                }
            }
        });
        self.timer.every(EVERY_MS, move || {
            if let Some(u) = me.upgrade() {
                u.tick();
            }
        });
    }

    fn tick(self: &Rc<Self>) {
        if self.mode() == "manuel" {
            return;
        }
        let me = self.clone();
        js::spawn(async move {
            me.check().await;
        });
    }

    /// Interroge le dépôt de diffusion. En mode auto, une nouvelle version est préparée aussitôt.
    pub async fn check(self: &Rc<Self>) -> J {
        self.refresh_local();
        let st = self.state();
        if !js::gb(&st, "installe") {
            return json!({ "ok": false, "error": "Jarvis n'est pas installé (version de développement) : pas de mise à jour à vérifier." });
        }
        let depot = js::gs(&st, "depot");
        let url = format!("https://api.github.com/repos/{depot}/tags?per_page=100");
        let res = crate::net::http().get(&url).header("Accept", "application/vnd.github+json").send().await;
        let tags: Vec<String> = match res {
            Ok(r) if r.status().is_success() => r.json::<J>().await.ok().and_then(|j| j.as_array().map(|a| a.iter().map(|t| js::gs(t, "name")).collect())).unwrap_or_default(),
            Ok(r) => return self.fail(format!("dépôt {depot} : réponse {}", r.status())),
            Err(e) => return self.fail(format!("dépôt {depot} injoignable : {e}")),
        };
        let found = newest(&tags, &js::gs(&st, "actuelle"), js::gs(&st, "canal") == "beta");
        let notes = match &found {
            Some(tag) => self.notes(&depot, tag, &js::gs(&st, "actuelle")).await,
            None => json!([]),
        };
        self.set(json!({ "disponible": found, "nouveautes": notes, "verifieLe": js::iso_now(), "erreur": J::Null }));
        if let Some(tag) = &found {
            (self.log)(format!("[mise à jour] version {tag} disponible"));
            let already = self.state()["pret"].as_str() == Some(tag.as_str()) || self.preparing.get();
            if self.mode() == "auto" && !already {
                self.prepare(tag);
            }
        }
        json!({ "ok": true, "etat": self.state() })
    }

    fn fail(&self, why: String) -> J {
        (self.log)(format!("[mise à jour] {why}"));
        self.set(json!({ "erreur": why, "verifieLe": js::iso_now() }));
        json!({ "ok": false, "error": why })
    }

    /// Nouveautés publiées depuis la version installée (ui/changelog.json du tag).
    async fn notes(&self, depot: &str, tag: &str, current: &str) -> J {
        let url = format!("https://raw.githubusercontent.com/{depot}/{tag}/ui/changelog.json");
        let Ok(r) = crate::net::http().get(&url).send().await else { return json!([]) };
        let Ok(log) = r.json::<J>().await else { return json!([]) };
        let cur = parse(current).map(|p| (p.0, p.1, p.2)).unwrap_or_default();
        let list: Vec<J> = js::arr(&log["versions"])
            .into_iter()
            .filter(|v| parse(&js::gs(v, "version")).is_some_and(|p| (p.0, p.1, p.2) > cur))
            .take(20)
            .map(|v| json!({ "version": v["version"], "titre": v["title"], "date": v["date"] }))
            .collect();
        json!(list)
    }

    /// Copie de installer.ps1 hors de app\ (remplacé pendant l'installation), lancée par Windows PowerShell.
    fn run_installer(&self, args: &[&str], visible: bool) -> Result<(), String> {
        let src = crate::paths::root().join("installer.ps1");
        let tmp = std::env::temp_dir().join("jarvis-installer.ps1");
        std::fs::copy(&src, &tmp).map_err(|e| format!("{} : {e}", src.display()))?;
        let script = tmp.to_string_lossy().to_string();
        let mut all: Vec<&str> = vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script];
        all.extend_from_slice(args);
        let ok = if visible { crate::proc::detached_visible("powershell.exe", &all) } else { crate::proc::detached("powershell.exe", &all) };
        if ok { Ok(()) } else { Err("PowerShell n'a pas pu être lancé".into()) }
    }

    /// Récupère et compile `tag` en arrière-plan (Jarvis reste ouvert). La fin se voit à build\pret.json.
    pub fn prepare(self: &Rc<Self>, tag: &str) -> J {
        if self.preparing.get() {
            return json!({ "ok": false, "error": "préparation déjà en cours" });
        }
        if let Err(e) = self.run_installer(&["-Preparer", "-Version", tag, "-Silencieux"], false) {
            return self.fail(e);
        }
        self.preparing.set(true);
        self.set(json!({ "preparation": true, "preparationDe": tag }));
        (self.log)(format!("[mise à jour] préparation de {tag} (compilation en arrière-plan)"));
        // Surveille la fin : pret.json écrit (succès), ou plus rien après 90 minutes (échec, voir installation.log).
        let me = Rc::downgrade(self);
        let tag = tag.to_string();
        let started = js::now_ms();
        js::spawn(async move {
            loop {
                js::sleep_ms(15_000).await;
                let Some(u) = me.upgrade() else { return };
                let done = ready().is_some_and(|r| js::gs(&r, "tag") == tag);
                if done || js::now_ms() - started > 90 * 60 * 1000 {
                    u.preparing.set(false);
                    u.refresh_local();
                    u.set(json!({ "preparation": false, "erreur": if done { J::Null } else { json!("la préparation n'a pas abouti (voir %LOCALAPPDATA%\\Jarvis\\installation.log)") } }));
                    if done {
                        (u.log)(format!("[mise à jour] {tag} prête : installée au prochain redémarrage"));
                    }
                    return;
                }
            }
        });
        json!({ "ok": true, "etat": self.state() })
    }

    /// Une version préparée peut-elle être installée ? La coquille lance ensuite « installer.ps1 -Appliquer » une fois
    /// Jarvis fermé (jarvis-shell › launch_pending_update), qui remplace le programme et relance Jarvis.
    pub fn apply(&self) -> J {
        if ready().is_none() {
            return json!({ "ok": false, "error": "aucune version préparée" });
        }
        json!({ "ok": true })
    }

    /// Ajout ou retrait d'un module (Administration › Modules) : immédiat pour un module d'interface seule,
    /// préparé puis installé au redémarrage pour un module à partie noyau.
    pub fn module(&self, add: bool, id: &str) -> J {
        if crate::paths::installed().is_none() {
            return json!({ "ok": false, "error": "version de développement : tous les modules sont déjà là" });
        }
        if !crate::re!(r"^[a-z0-9][a-z0-9-]{0,39}$").is_match(id) {
            return json!({ "ok": false, "error": "module invalide" });
        }
        match self.run_installer(&[if add { "-AjouterModule" } else { "-RetirerModule" }, id], true) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => self.fail(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn version_la_plus_recente_du_canal() {
        let t = tags(&["v4.101.0", "v4.102.0-beta", "v4.100.3", "modeles-1", "v4.99.9"]);
        assert_eq!(newest(&t, "4.101.0", false), None);
        assert_eq!(newest(&t, "4.100.3", false).as_deref(), Some("v4.101.0"));
        assert_eq!(newest(&t, "4.101.0", true).as_deref(), Some("v4.102.0-beta"));
        // La stable du même numéro remplace la bêta installée, jamais l'inverse.
        let t2 = tags(&["v4.102.0-beta", "v4.102.0"]);
        assert_eq!(newest(&t2, "4.102.0-beta", true).as_deref(), Some("v4.102.0"));
        assert_eq!(newest(&t2, "4.102.0", true), None);
        // 4.110 > 4.99 (comparaison numérique, pas alphabétique).
        assert_eq!(newest(&tags(&["v4.110.0"]), "4.99.0", false).as_deref(), Some("v4.110.0"));
    }
}
