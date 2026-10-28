// Modules liés au noyau : la partie Rust (modules/<id>/core) de chaque module qui en a une.
// Ajouter un module avec une partie noyau : dépendance facultative et feature dans Cargo.toml, et une ligne ici.
// Une installation ne lie que les modules choisis (features) ; le développement les lie tous (default).
// Un module sans partie noyau (interface seule) n'apparaît pas ici : sa fiche et son dossier ui/ suffisent.

use std::rc::Rc;

use jarvis_core::Module;

#[allow(unused_mut)]
pub fn all() -> Vec<Rc<dyn Module>> {
    let mut v: Vec<Rc<dyn Module>> = vec![];
    #[cfg(feature = "audio")]
    v.push(jarvis_mod_audio::module());
    v
}

/// Modules à partie noyau liés dans cette compilation (ids), pour l'administration et la mise à jour.
pub fn linked() -> Vec<String> {
    all().iter().map(|m| m.id().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    /// Chaque module du dépôt qui déclare une partie noyau (« core » dans module.json) a sa feature et, celle-ci active
    /// (compilation par défaut), est lié sous son id ; chaque module lié a sa fiche.
    #[test]
    fn modules_lies_complets() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("modules");
        let linked: Vec<String> = super::all().iter().map(|m| m.id().to_string()).collect();
        let mut declared = vec![];
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('_') || !e.path().join("module.json").is_file() {
                continue;
            }
            let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.path().join("module.json")).unwrap()).unwrap();
            if m["core"].as_str().map(|s| !s.is_empty()).unwrap_or(false) {
                assert!(e.path().join("core").join("Cargo.toml").is_file(), "{name} : « core » déclaré mais pas de core/Cargo.toml");
                let manifest = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
                assert!(manifest.contains(&format!("\n{name} = [\"dep:")), "{name} : pas de feature « {name} » dans crates/jarvis-modules/Cargo.toml");
                let app = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("jarvis-app").join("Cargo.toml")).unwrap();
                assert!(app.contains(&format!("\n{name} = [\"jarvis-modules/{name}\"]")), "{name} : feature à relayer dans crates/jarvis-app/Cargo.toml");
                assert!(linked.contains(&name), "{name} : partie noyau non liée (crates/jarvis-modules, feature active par défaut)");
                declared.push(name);
            }
        }
        for id in &linked {
            assert!(declared.contains(id), "{id} : lié mais sans fiche module.json qui déclare « core »");
        }
    }

    /// Installation à la carte : chaque dépendance existe, un module du socle ne dépend que du socle, et le socle
    /// contient la card Conversation.
    #[test]
    fn dependances_et_socle() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("modules");
        let mut all = std::collections::HashMap::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('_') || !e.path().join("module.json").is_file() {
                continue;
            }
            let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.path().join("module.json")).unwrap()).unwrap();
            jarvis_core::modules::validate(&m, &name).unwrap();
            all.insert(name, m);
        }
        let socle = |id: &str| all.get(id).is_some_and(|m| m["socle"] == true);
        assert!(socle("conversation"));
        for (id, m) in &all {
            for d in m["dependances"].as_array().into_iter().flatten().filter_map(|d| d.as_str()) {
                assert!(all.contains_key(d), "{id} dépend de « {d} », module inconnu");
                assert!(!socle(id) || socle(d), "{id} est du socle mais dépend de « {d} », qui ne l'est pas");
            }
        }
    }
}
