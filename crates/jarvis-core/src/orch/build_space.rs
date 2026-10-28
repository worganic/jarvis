// Place occupée par les compilations de Jarvis (target/) : mesure en arrière-plan, alerte au-delà du
// seuil (réglage « buildSpace » : alert, alertGb) et effacement des anciennes copies des crates depuis
// l'administration (Optimisation). Le travail sur le disque se fait hors du fil du noyau.
// Calcul et règles de tri : crate::build_sweep.

use std::rc::Rc;

use serde_json::json;

use super::Orchestrator;
use crate::js::{self, J};

const FIRST_SCAN_MS: u64 = 3 * 60_000;
const SCAN_EVERY_MS: u64 = 6 * 3600 * 1000;
/// Une alerte au plus par jour, même après une relance.
const ALERT_EVERY_MS: i64 = 24 * 3600 * 1000;
const STATE_FILE: &str = "build-space.json";
const GB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn gb(bytes: &J) -> f64 {
    js::num(bytes).unwrap_or(0.0) / GB
}

/// « 52 Go », « 3,4 Go ».
pub fn gb_label(bytes: &J) -> String {
    let g = gb(bytes);
    if g >= 10.0 { format!("{} Go", g.round()) } else { format!("{:.1} Go", g).replace('.', ",") }
}

impl Orchestrator {
    fn build_space_file(&self) -> std::path::PathBuf {
        self.data_dir.join(STATE_FILE)
    }

    /// Dernière mesure connue (aussi après une relance) et seuil d'alerte : affichée dans l'administration.
    pub fn build_space_state(&self) -> J {
        let mut last = self.build_space.borrow().clone();
        if last.is_null() {
            last = js::read_json(&self.build_space_file()).map(|s| s["mesure"].clone()).unwrap_or(J::Null);
            *self.build_space.borrow_mut() = last.clone();
        }
        let cfg = self.config.get("buildSpace");
        let limit = js::num(&cfg["alertGb"]).unwrap_or(40.0);
        let over = !last.is_null() && gb(&last["octets"]) >= limit;
        json!({ "mesure": last, "seuilGo": limit, "alerte": js::truthy(&cfg["alert"]), "depasse": over, "occupe": self.build_space_busy.get() })
    }

    pub fn start_build_watch(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        js::set_timeout(FIRST_SCAN_MS, {
            let me = me.clone();
            move || {
                if let Some(o) = me.upgrade() {
                    js::spawn(async move {
                        o.build_space_scan(true).await;
                    });
                }
            }
        });
        self.build_space_timer.every(SCAN_EVERY_MS, move || {
            if let Some(o) = me.upgrade() {
                js::spawn(async move {
                    o.build_space_scan(true).await;
                });
            }
        });
    }

    /// Mesure target/ ; `may_alert` : alerte si le seuil est dépassé (pas lors d'une mesure demandée).
    pub async fn build_space_scan(&self, may_alert: bool) -> J {
        if self.build_space_busy.replace(true) {
            return json!({ "ok": false, "error": "Une mesure ou un nettoyage est déjà en cours." });
        }
        let target = crate::build_sweep::target_dir();
        let r = tokio::task::spawn_blocking(move || crate::build_sweep::scan(&target)).await.unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }));
        self.build_space_busy.set(false);
        if !js::gb(&r, "ok") {
            return r;
        }
        *self.build_space.borrow_mut() = r.clone();
        let mut saved = js::read_json(&self.build_space_file()).unwrap_or(json!({}));
        saved["mesure"] = r.clone();
        if may_alert && self.build_space_alert(&r, js::num(&saved["alerteLe"]).unwrap_or(0.0) as i64) {
            saved["alerteLe"] = json!(js::now_ms());
        }
        let _ = js::write_json(&self.build_space_file(), &saved);
        self.emit(json!({ "type": "build_space", "state": self.build_space_state() }));
        json!({ "ok": true, "state": self.build_space_state() })
    }

    /// Alerte (parole et card) quand les compilations dépassent le seuil ; rend vrai si elle a été émise.
    fn build_space_alert(&self, r: &J, last_alert: i64) -> bool {
        let cfg = self.config.get("buildSpace");
        let limit = js::num(&cfg["alertGb"]).unwrap_or(40.0);
        if !js::truthy(&cfg["alert"]) || gb(&r["octets"]) < limit || js::now_ms() - last_alert < ALERT_EVERY_MS {
            return false;
        }
        let total = gb_label(&r["octets"]);
        let old = gb_label(&r["anciennes"]);
        self.log(format!("[compilation] target/ occupe {total} (seuil {limit} Go), dont {old} d'anciennes copies"));
        self.notify(json!({
            "text": format!("Les compilations de Jarvis occupent {total} sur le disque, dont {old} d'anciennes copies. Tu peux les effacer dans l'administration, section Optimisation."),
            "title": "Espace disque", "source": "compilation", "ttl": 600000,
            "rows": [
                { "icon": "⚠", "tone": "danger", "title": format!("Compilations : {total}"), "sub": format!("seuil {limit} Go · dont {old} d'anciennes copies") },
                { "icon": "◐", "title": "Ouvrir l'administration", "say": "Ouvre l'administration" }
            ],
            "footer": "Administration › Optimisation › Espace disque des compilations"
        }));
        true
    }

    /// Efface les anciennes copies des crates, puis remesure.
    pub async fn build_space_sweep(&self) -> J {
        if self.build_space_busy.replace(true) {
            return json!({ "ok": false, "error": "Une mesure ou un nettoyage est déjà en cours." });
        }
        let target = crate::build_sweep::target_dir();
        let r = tokio::task::spawn_blocking(move || crate::build_sweep::sweep(&target)).await.unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }));
        self.build_space_busy.set(false);
        if !js::gb(&r, "ok") {
            return r;
        }
        self.log(format!("[compilation] anciennes copies effacées : {} ({} élément(s), {} verrouillé(s))", gb_label(&r["liberes"]), js::s(&r["effaces"]), js::s(&r["echecs"])));
        self.record("compilation_nettoyee", json!({ "liberes": r["liberes"], "effaces": r["effaces"], "origine": "administration" }));
        let scan = self.build_space_scan(false).await;
        json!({ "ok": true, "liberes": r["liberes"], "effaces": r["effaces"], "echecs": r["echecs"], "state": scan["state"] })
    }
}
