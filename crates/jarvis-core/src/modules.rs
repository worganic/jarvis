// Registre des modules (modules/<id>/module.json) : fiches lues sur le disque, état activé / désactivé
// (data/modules.json), liste servie aux fenêtres (le HUD charge l'interface des modules actifs) et à
// l'administration (panneau « Modules »).
//
// Un module sans partie noyau s'ajoute ou s'enlève sans recompiler : il suffit de déposer ou de supprimer son
// dossier. Une partie noyau (modules/<id>/core) n'existe que si elle est liée au binaire (crates/jarvis-modules).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::js::{self, J};

/// Identifiant valide : le nom du dossier, en minuscules (lettres, chiffres, tirets).
fn id_ok(id: &str) -> bool {
    crate::re!(r"^[a-z][a-z0-9-]{0,39}$").is_match(id)
}

/// Chemin relatif sûr d'un fichier du module (sous ui/, sans remontée).
fn ui_path_ok(p: &str) -> bool {
    p.starts_with("ui/") && !p.split('/').any(|s| s.is_empty() || s == ".." || s.contains(':') || s.contains('\\'))
}

/// Cards d'un module : la principale (« card ») et les autres (« autresCards » : listes ouvertes par le noyau…).
pub fn cards_of(m: &J) -> Vec<&J> {
    let mut out: Vec<&J> = vec![];
    if !m["card"].is_null() {
        out.push(&m["card"]);
    }
    if let Some(list) = m["autresCards"].as_array() {
        out.extend(list.iter());
    }
    out
}

fn validate_card(c: &J, folder: &str) -> Result<(), String> {
    let t = c["type"].as_i64().unwrap_or(0);
    let n = c["numero"].as_i64().unwrap_or(0);
    let ok = match t {
        1 => (1..=99).contains(&n),
        2 => (100..=199).contains(&n),
        _ => false,
    };
    if !ok {
        return Err(format!("{folder} : card de type {t} avec le numéro {n} (type 1 : 1 à 99, type 2 : 100 à 199)"));
    }
    // Emplacement de numéro d'une card ouverte par le noyau (orch/cards.rs › slot_of : « note », « tool:Terminal »…).
    if !c["emplacement"].is_null() && !crate::re!(r"^[A-Za-z][\w :-]{0,60}$").is_match(&js::gs(c, "emplacement")) {
        return Err(format!("{folder} : card.emplacement invalide"));
    }
    // Clé de la card dans le HUD (J.CARD_NO) quand elle diffère de l'id (ex. « hw » pour le module Matériel).
    if !c["cle"].is_null() && !crate::re!(r"^[a-z][a-z0-9_-]{0,39}$").is_match(&js::gs(c, "cle")) {
        return Err(format!("{folder} : card.cle invalide"));
    }
    Ok(())
}

/// Vérifie une fiche ; `folder` est le nom du dossier du module.
pub fn validate(m: &J, folder: &str) -> Result<(), String> {
    let id = js::gs(m, "id");
    if !id_ok(&id) {
        return Err(format!("{folder} : « id » invalide (minuscules, chiffres, tirets)"));
    }
    if id != folder {
        return Err(format!("{folder} : « id » ({id}) différent du nom du dossier"));
    }
    if js::gs(m, "nom").trim().is_empty() {
        return Err(format!("{folder} : « nom » manquant"));
    }
    for c in cards_of(m) {
        validate_card(c, folder)?;
    }
    // Kinds des cards affichées par le module (événement « widget ») : ignorés quand il est désactivé.
    if !m["widgets"].is_null() && !m["widgets"].as_array().map(|a| a.iter().all(|k| k.as_str().map(|k| crate::re!(r"^[a-z][a-z0-9_]{0,39}(?:\.[a-z][a-z0-9_]{0,39})?$").is_match(k)).unwrap_or(false))).unwrap_or(false) {
        return Err(format!("{folder} : « widgets » doit être une liste de noms (« note », « tool.collection »…)"));
    }
    if !m["actions"].is_null() {
        let Some(list) = m["actions"].as_array() else { return Err(format!("{folder} : « actions » doit être une liste")) };
        for a in list {
            let say = !js::gs(a, "dire").trim().is_empty();
            let call = a["appel"].as_array().map(|c| !c.is_empty() && c.iter().all(|x| x.is_string())).unwrap_or(false);
            if js::gs(a, "titre").trim().is_empty() || say == call {
                return Err(format!("{folder} : chaque action a un « titre » et soit « dire » (phrase), soit « appel » (méthode et arguments)"));
            }
        }
    }
    // Installation à la carte (installer.ps1) : module indispensable, et modules dont celui-ci a besoin.
    if !m["socle"].is_null() && !m["socle"].is_boolean() {
        return Err(format!("{folder} : « socle » doit valoir true ou false"));
    }
    if !m["dependances"].is_null() && !m["dependances"].as_array().map(|a| a.iter().all(|d| d.as_str().is_some_and(id_ok))).unwrap_or(false) {
        return Err(format!("{folder} : « dependances » doit être une liste d'ids de modules"));
    }
    for key in ["scripts", "styles", "es"] {
        let list = &m["ui"][key];
        if list.is_null() {
            continue;
        }
        let Some(arr) = list.as_array() else { return Err(format!("{folder} : ui.{key} doit être une liste")) };
        for p in arr {
            let p = p.as_str().unwrap_or("");
            if !ui_path_ok(p) {
                return Err(format!("{folder} : ui.{key} « {p} » doit être un fichier de ui/"));
            }
        }
    }
    Ok(())
}

/// Préfixe obligatoire des modules créés par l'utilisateur (data/user-modules/<id>/).
pub const USER_PREFIX: &str = "perso-";

/// Dossier des modules créés par l'utilisateur.
pub fn user_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("user-modules")
}

/// Fiches trouvées dans `dir` (dossiers commençant par « _ » ignorés : modèles), et erreurs.
pub fn scan(dir: &Path) -> (Vec<J>, Vec<String>) {
    scan_roots(&[(dir.to_path_buf(), false)])
}

/// Fiches trouvées dans plusieurs dossiers ; le booléen indique un dossier de modules utilisateur (id « perso-… »
/// imposé, fiche marquée « utilisateur »). Les modules livrés passent avant : ils gardent leur numéro de card.
pub fn scan_roots(roots: &[(PathBuf, bool)]) -> (Vec<J>, Vec<String>) {
    let mut found: BTreeMap<(bool, String), J> = BTreeMap::new();
    let mut errors = vec![];
    for (dir, user) in roots {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let folder = e.file_name().to_string_lossy().to_string();
            if !path.is_dir() || folder.starts_with('_') || folder.starts_with('.') {
                continue;
            }
            let file = path.join("module.json");
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let mut m: J = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(err) => {
                    errors.push(format!("{folder} : module.json illisible ({err})"));
                    continue;
                }
            };
            if *user {
                if !folder.starts_with(USER_PREFIX) {
                    errors.push(format!("{folder} : un module utilisateur s'appelle « {USER_PREFIX}… »"));
                    continue;
                }
                if found.keys().any(|(_, f)| *f == folder) {
                    errors.push(format!("{folder} : nom déjà pris par un module livré"));
                    continue;
                }
                m["utilisateur"] = json!(true);
            }
            match validate(&m, &folder) {
                Ok(()) => {
                    found.insert((*user, folder), m);
                }
                Err(err) => errors.push(err),
            }
        }
    }
    // Deux modules ne prennent jamais le même numéro de card : le second est écarté.
    let mut numbers = HashSet::new();
    let mut out = vec![];
    for ((_, folder), m) in found {
        let taken: Vec<i64> = cards_of(&m).iter().filter_map(|c| c["numero"].as_i64()).collect();
        if let Some(n) = taken.iter().find(|n| numbers.contains(*n)) {
            errors.push(format!("{folder} : numéro de card {n} déjà pris par un autre module"));
            continue;
        }
        numbers.extend(taken);
        out.push(m);
    }
    (out, errors)
}

/// Numéros par défaut des cards ouvertes par le noyau, d'après les fiches (emplacement → numéro) ; lus par
/// orch/cards.rs › default_slot_number avant ses valeurs de repli.
static SLOTS: std::sync::RwLock<Option<std::collections::HashMap<String, i64>>> = std::sync::RwLock::new(None);

pub fn slot_default(slot: &str) -> Option<i64> {
    SLOTS.read().ok()?.as_ref()?.get(slot).copied()
}

pub fn number_default_slot(n: i64) -> Option<String> {
    if let Some(s) = SLOTS.read().ok().and_then(|g| g.as_ref()?.iter().find(|(_, num)| **num == n).map(|(k, _)| k.clone())) {
        return Some(s);
    }
    let fallback = match n {
        100 => "bureau",
        101 => "tool:Terminal",
        102 => "mail",
        103 => "agenda",
        104 => "note",
        105 => "meeting",
        106 => "tool:Mes listes",
        107 => "tool:notes",
        108 => "tool:reunions",
        109 => "tool:projets",
        110 => "savoir",
        111 => "devant",
        112 => "diction",
        113 => "checkup",
        114 => "creer-module",
        115 => "agents",
        120 => "nomade_view",
        124 => "amis_view",
        _ => return None,
    };
    Some(fallback.to_string())
}

fn publish_slots(mods: &[J]) {
    let map = mods
        .iter()
        .flat_map(cards_of)
        .filter_map(|c| {
            let k = [js::gs(c, "emplacement"), js::gs(c, "cle")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
            Some((k, c["numero"].as_i64()?))
        })
        .filter(|(k, _)| !k.is_empty())
        .collect();
    if let Ok(mut w) = SLOTS.write() {
        *w = Some(map);
    }
}

pub struct Registry {
    dir: PathBuf,
    user_dir: PathBuf,
    state_file: PathBuf,
    /// Modules dont la partie noyau est liée à ce binaire.
    linked: Vec<String>,
    /// Choix de l'administration : id → actif.
    state: RefCell<serde_json::Map<String, Value>>,
}

impl Registry {
    pub fn new(dir: PathBuf, data_dir: &Path, linked: Vec<String>) -> Self {
        let state_file = data_dir.join("modules.json");
        let state = std::fs::read_to_string(&state_file)
            .ok()
            .and_then(|t| serde_json::from_str::<J>(&t).ok())
            .and_then(|v| v["actifs"].as_object().cloned())
            .unwrap_or_default();
        let user_dir = user_dir(data_dir);
        let reg = Self { dir, user_dir, state_file, linked, state: RefCell::new(state) };
        publish_slots(&reg.scan_all().0);
        reg
    }

    /// Modules livrés et modules créés par l'utilisateur.
    pub fn scan_all(&self) -> (Vec<J>, Vec<String>) {
        scan_roots(&[(self.dir.clone(), false), (self.user_dir.clone(), true)])
    }

    fn enabled_in(&self, m: &J) -> bool {
        let id = js::gs(m, "id");
        self.state.borrow().get(&id).and_then(Value::as_bool).unwrap_or(m["parDefaut"].as_bool().unwrap_or(true))
    }

    /// Module actif : sa fiche est présente et valide, et il n'est pas désactivé.
    pub fn is_enabled(&self, id: &str) -> bool {
        let (mods, _) = self.scan_all();
        mods.iter().any(|m| js::gs(m, "id") == id && self.enabled_in(m))
    }

    /// Liste pour les fenêtres et l'administration ; les chemins de l'interface sont relatifs à la racine servie.
    pub fn list(&self) -> J {
        let (mods, errors) = self.scan_all();
        let items: Vec<J> = mods
            .iter()
            .map(|m| {
                let id = js::gs(m, "id");
                let files = |key: &str| -> Vec<String> {
                    m["ui"][key].as_array().map(|a| a.iter().filter_map(|p| p.as_str()).map(|p| format!("modules/{id}/{p}")).collect()).unwrap_or_default()
                };
                let core = js::gs(m, "core");
                json!({
                    "id": id,
                    "nom": m["nom"],
                    "version": m["version"],
                    "description": m["description"],
                    "card": m["card"],
                    "autresCards": m["autresCards"],
                    "actions": m["actions"],
                    "widgets": m["widgets"],
                    "scripts": files("scripts"),
                    "styles": files("styles"),
                    "es": files("es"),
                    "noyau": !core.is_empty(),
                    "lie": self.linked.contains(&id),
                    "actif": self.enabled_in(m),
                    "parDefaut": m["parDefaut"].as_bool().unwrap_or(true),
                    "utilisateur": m["utilisateur"].as_bool().unwrap_or(false),
                    "revision": m["revision"],
                })
            })
            .collect();
        json!({ "ok": true, "modules": items, "erreurs": errors })
    }

    /// Active ou désactive un module (choix enregistré dans data/modules.json).
    pub fn set_enabled(&self, id: &str, on: bool) -> J {
        let (mods, _) = self.scan_all();
        let Some(m) = mods.iter().find(|m| js::gs(m, "id") == id) else {
            return json!({ "ok": false, "error": format!("module inconnu : {id}") });
        };
        self.state.borrow_mut().insert(id.to_string(), json!(on));
        let body = json!({ "actifs": Value::Object(self.state.borrow().clone()) });
        if let Err(e) = std::fs::write(&self.state_file, serde_json::to_string_pretty(&body).unwrap_or_default()) {
            return json!({ "ok": false, "error": format!("enregistrement impossible : {e}") });
        }
        let core = !js::gs(m, "core").is_empty();
        json!({ "ok": true, "id": id, "actif": on, "relance": core, "liste": self.list() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_modules() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("modules")
    }

    #[test]
    fn fiches_du_depot_valides() {
        let (mods, errors) = scan(&repo_modules());
        assert!(errors.is_empty(), "fiches de modules invalides : {errors:?}");
        assert!(mods.iter().any(|m| m["id"] == "audio"), "module Audio introuvable");
        assert!(mods.iter().any(|m| m["id"] == "materiel"), "module Matériel introuvable");
    }

    #[test]
    fn modele_valide() {
        let dir = repo_modules().join("_modele");
        let m: J = serde_json::from_str(&std::fs::read_to_string(dir.join("module.json")).unwrap()).unwrap();
        // Le modèle se copie sous un autre nom : on le vérifie sous son propre id.
        let id = js::gs(&m, "id");
        validate(&m, &id).unwrap();
    }

    #[test]
    fn refus() {
        let ok = json!({ "id": "essai", "nom": "Essai", "card": { "type": 2, "numero": 150 }, "ui": { "scripts": ["ui/essai.js"] } });
        assert!(validate(&ok, "essai").is_ok());
        assert!(validate(&ok, "autre").is_err());
        assert!(validate(&json!({ "id": "Essai", "nom": "x" }), "Essai").is_err());
        assert!(validate(&json!({ "id": "essai", "nom": "x", "card": { "type": 1, "numero": 150 } }), "essai").is_err());
        assert!(validate(&json!({ "id": "essai", "nom": "x", "ui": { "scripts": ["../ui/app.js"] } }), "essai").is_err());
        assert!(validate(&json!({ "id": "essai", "nom": "x", "ui": { "scripts": ["core/src/lib.rs"] } }), "essai").is_err());
    }

    #[test]
    fn modules_utilisateur_apres_les_livres() {
        let tmp = std::env::temp_dir().join(format!("jarvis-mod-user-{}", js::now_ms()));
        let shipped = tmp.join("modules").join("essai");
        let mine = user_dir(&tmp).join("perso-compteur");
        let bad = user_dir(&tmp).join("autre");
        for d in [&shipped, &mine, &bad] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(shipped.join("module.json"), r#"{ "id": "essai", "nom": "Essai", "card": { "type": 2, "numero": 150 } }"#).unwrap();
        // Même numéro de card que le module livré : écarté ; le préfixe « perso- » est imposé.
        std::fs::write(mine.join("module.json"), r#"{ "id": "perso-compteur", "nom": "Compteur", "card": { "type": 2, "numero": 150 } }"#).unwrap();
        std::fs::write(bad.join("module.json"), r#"{ "id": "autre", "nom": "Autre", "card": { "type": 2, "numero": 151 } }"#).unwrap();
        let r = Registry::new(tmp.join("modules"), &tmp, vec![]);
        let l = r.list();
        let ids: Vec<String> = l["modules"].as_array().unwrap().iter().map(|m| js::gs(m, "id")).collect();
        assert_eq!(ids, vec!["essai".to_string()]);
        assert_eq!(l["erreurs"].as_array().unwrap().len(), 2);
        std::fs::write(mine.join("module.json"), r#"{ "id": "perso-compteur", "nom": "Compteur", "card": { "type": 2, "numero": 151 } }"#).unwrap();
        let l = r.list();
        let user = l["modules"].as_array().unwrap().iter().find(|m| m["id"] == "perso-compteur").cloned().unwrap();
        assert_eq!(user["utilisateur"], json!(true));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn activation_enregistree() {
        let tmp = std::env::temp_dir().join(format!("jarvis-mod-{}", js::now_ms()));
        let mods = tmp.join("modules").join("essai");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::write(mods.join("module.json"), r#"{ "id": "essai", "nom": "Essai", "parDefaut": false }"#).unwrap();
        let r = Registry::new(tmp.join("modules"), &tmp, vec![]);
        assert!(!r.is_enabled("essai"));
        assert!(js::gb(&r.set_enabled("essai", true), "ok"));
        let r2 = Registry::new(tmp.join("modules"), &tmp, vec![]);
        assert!(r2.is_enabled("essai"));
        assert!(!js::gb(&r2.set_enabled("absent", true), "ok"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
