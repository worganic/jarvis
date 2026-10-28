// Table des zones d'accès déclarées de Jarvis (orchestrator/zones.js).
// Zones nommées avec règles de lecture/écriture distinctes, autorisations temporaires d'écriture.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::js::{self, now_ms, J};

#[derive(Clone)]
pub struct TempAuth {
    pub zone_id: String,
    pub zone_name: String,
    pub granted_at: i64,
    pub expires_at: i64,
    pub duration_minutes: i64,
}

pub struct Zones {
    file: PathBuf,
    default_workspace: PathBuf,
    pub zones: RefCell<Vec<J>>,
    temporary: RefCell<HashMap<String, TempAuth>>,
}

/// path.resolve() : chemin absolu normalisé.
pub fn resolve(p: &str) -> String {
    let path = Path::new(p);
    std::path::absolute(path).map(|a| a.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string())
}

fn norm(p: &str) -> String {
    resolve(p).to_lowercase().replace('/', "\\")
}

impl Zones {
    pub fn new(file: PathBuf, default_workspace: PathBuf) -> Self {
        let z = Self { file, default_workspace, zones: RefCell::new(vec![]), temporary: RefCell::new(HashMap::new()) };
        z.load();
        z
    }

    fn defaults(&self) -> Vec<J> {
        let home = js::env("USERPROFILE").or_else(|| js::env("HOME")).unwrap_or_else(|| "C:\\Users\\Default".into());
        let home = PathBuf::from(home);
        vec![
            json!({ "id": "workspace", "name": "Workspace Jarvis", "path": resolve(&self.default_workspace.to_string_lossy()), "read": "free", "write": "free" }),
            json!({ "id": "desktop", "name": "Bureau", "path": home.join("Desktop").to_string_lossy(), "read": "free", "write": "code" }),
            json!({ "id": "documents", "name": "Documents", "path": home.join("Documents").to_string_lossy(), "read": "free", "write": "code" }),
            json!({ "id": "downloads", "name": "Téléchargements", "path": home.join("Downloads").to_string_lossy(), "read": "free", "write": "code" }),
            json!({ "id": "projects", "name": "Projets (E:\\www)", "path": "E:\\www", "read": "free", "write": "code" }),
        ]
    }

    pub fn load(&self) {
        if let Some(Value::Array(mut a)) = js::read_json(&self.file) {
            if !a.is_empty() {
                // Zone « workspace » restée sur un ancien dossier (copie depuis la v2/v3) : elle suit le
                // dossier de travail réel, sinon les écritures des agents y demandent un accord.
                let ws = resolve(&self.default_workspace.to_string_lossy());
                let stale = a.iter_mut().find(|z| js::gs(z, "id") == "workspace").filter(|z| norm(&js::gs(z, "path")) != norm(&ws));
                let moved = stale.map(|z| z["path"] = json!(ws)).is_some();
                *self.zones.borrow_mut() = a;
                if moved {
                    self.save();
                }
                return;
            }
        }
        *self.zones.borrow_mut() = self.defaults();
        self.save();
    }

    pub fn save(&self) {
        let _ = js::write_json(&self.file, &Value::Array(self.zones.borrow().clone()));
    }

    pub fn list(&self) -> Vec<J> {
        self.zones.borrow().clone()
    }

    /// Zone la plus spécifique contenant le chemin cible.
    pub fn find_zone(&self, target: &str) -> Option<J> {
        if target.is_empty() {
            return None;
        }
        let normalized = norm(target);
        let mut best: Option<J> = None;
        let mut longest: i64 = -1;
        for zone in self.zones.borrow().iter() {
            let zp = js::gs(zone, "path");
            if zp.is_empty() {
                continue;
            }
            let zn = norm(&zp);
            let prefix = if zn.ends_with('\\') { zn.clone() } else { format!("{zn}\\") };
            if (normalized == zn || normalized.starts_with(&prefix)) && zn.len() as i64 > longest {
                longest = zn.len() as i64;
                best = Some(zone.clone());
            }
        }
        best
    }

    /// Autorisation temporaire d'écriture sans validation (« autorise l'écriture dans … pendant 30 minutes »).
    pub fn grant_temporary(&self, zone_id_or_name: &str, duration_minutes: f64) -> Option<J> {
        let target = zone_id_or_name.to_lowercase().trim().to_string();
        let zones = self.zones.borrow();
        let mut zone = None;
        if !target.is_empty() {
            zone = zones.iter().find(|z| js::gs(z, "id").to_lowercase() == target || js::gs(z, "name").to_lowercase().contains(&target)).cloned();
        }
        if zone.is_none() {
            zone = zones
                .iter()
                .find(|z| js::gs(z, "id") == "projects")
                .or_else(|| zones.iter().find(|z| js::gs(z, "id") == "workspace"))
                .or_else(|| zones.first())
                .cloned();
        }
        let zone = zone?;
        let minutes = (if duration_minutes.is_finite() && duration_minutes != 0.0 { duration_minutes } else { 60.0 }).clamp(1.0, 720.0) as i64;
        let expires_at = now_ms() + minutes * 60 * 1000;
        let id = js::gs(&zone, "id");
        self.temporary.borrow_mut().insert(
            id.clone(),
            TempAuth { zone_id: id, zone_name: js::gs(&zone, "name"), granted_at: now_ms(), expires_at, duration_minutes: minutes },
        );
        Some(json!({ "zone": zone, "durationMinutes": minutes, "expiresAt": js::iso_of(expires_at) }))
    }

    pub fn is_temporarily_granted(&self, zone_id: &str) -> bool {
        let mut t = self.temporary.borrow_mut();
        match t.get(zone_id) {
            None => false,
            Some(a) if now_ms() > a.expires_at => {
                t.remove(zone_id);
                false
            }
            Some(_) => true,
        }
    }

    pub fn list_temporary(&self) -> Vec<J> {
        let now = now_ms();
        let mut t = self.temporary.borrow_mut();
        t.retain(|_, a| now <= a.expires_at);
        t.values()
            .map(|a| {
                json!({
                    "zoneId": a.zone_id, "zoneName": a.zone_name, "grantedAt": a.granted_at, "expiresAt": a.expires_at,
                    "durationMinutes": a.duration_minutes,
                    "remainingMinutes": (((a.expires_at - now) as f64) / 60000.0).round().max(1.0) as i64
                })
            })
            .collect()
    }

    pub fn revoke_temporary(&self, zone_id_or_name: &str) -> bool {
        let target = zone_id_or_name.to_lowercase();
        let mut t = self.temporary.borrow_mut();
        let key = t.iter().find(|(id, a)| id.to_lowercase() == target || a.zone_name.to_lowercase().contains(&target)).map(|(k, _)| k.clone());
        match key {
            Some(k) => {
                t.remove(&k);
                true
            }
            None => false,
        }
    }

    /// Permission d'accès : « free », « code » ou « forbidden ».
    pub fn check(&self, target: &str, access: &str) -> String {
        if let Some(zone) = self.find_zone(target) {
            if access == "write" {
                if self.is_temporarily_granted(&js::gs(&zone, "id")) {
                    return "free".into();
                }
                let w = js::gs(&zone, "write");
                return if w.is_empty() { "code".into() } else { w };
            }
            let r = js::gs(&zone, "read");
            return if r.is_empty() { "free".into() } else { r };
        }
        // Hors des zones déclarées : lecture avec code, écriture interdite.
        if access == "write" { "forbidden".into() } else { "code".into() }
    }

    pub fn add(&self, name: &str, target: &str, read: &str, write: &str) -> J {
        let id = crate::re!(r"[^a-z0-9]").replace_all(&name.to_lowercase(), "_").into_owned();
        let resolved = resolve(target);
        let ok = |v: &str| ["free", "code", "forbidden"].contains(&v);
        let entry = json!({
            "id": id, "name": name, "path": resolved,
            "read": if ok(read) { read } else { "free" },
            "write": if ok(write) { write } else { "code" }
        });
        {
            let mut zones = self.zones.borrow_mut();
            let existing = zones.iter().position(|z| js::gs(z, "id") == id || (!js::gs(z, "path").is_empty() && resolve(&js::gs(z, "path")) == resolved));
            match existing {
                Some(i) => zones[i] = entry.clone(),
                None => zones.push(entry.clone()),
            }
        }
        self.save();
        entry
    }

    pub fn remove(&self, id_or_name: &str) -> Option<J> {
        let target = id_or_name.to_lowercase();
        let removed = {
            let mut zones = self.zones.borrow_mut();
            let idx = zones.iter().position(|z| js::gs(z, "id") == target || js::gs(z, "name").to_lowercase() == target);
            match idx {
                Some(i) if js::gs(&zones[i], "id") != "workspace" => Some(zones.remove(i)),
                _ => None,
            }
        };
        if removed.is_some() {
            self.save();
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jarvis-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn la_zone_workspace_suit_le_vrai_dossier_de_travail() {
        let d = temp("zones");
        let file = d.join("zones.json");
        let ancien = d.join("ancien-workspace");
        let reel = d.join("workspace");
        std::fs::write(&file, serde_json::to_string(&json!([{ "id": "workspace", "name": "W", "path": ancien.to_string_lossy(), "read": "free", "write": "free" }])).unwrap()).unwrap();
        let z = Zones::new(file.clone(), reel.clone());
        assert_eq!(z.check(&reel.join("note.md").to_string_lossy(), "write"), "free");
        // Le fichier est corrigé pour la prochaine fois.
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(norm(saved[0]["path"].as_str().unwrap()), norm(&reel.to_string_lossy()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hors_des_zones_lecture_avec_code_ecriture_interdite() {
        let d = temp("zones2");
        let z = Zones::new(d.join("zones.json"), d.join("workspace"));
        assert_eq!(z.check("Z:\\inconnu\\fichier.txt", "write"), "forbidden");
        assert_eq!(z.check("Z:\\inconnu\\fichier.txt", "read"), "code");
        let _ = std::fs::remove_dir_all(&d);
    }
}
