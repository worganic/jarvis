// Emplacements de Jarvis v4 : racine du projet (dossier qui contient ui/ et Cargo.toml), données,
// espace de travail. JARVIS_ROOT, JARVIS_DATA_DIR et JARVIS_WORKSPACE les remplacent (tests).
//
// Les données de l'utilisateur vivent hors du programme : %LOCALAPPDATA%\Jarvis\donnees (même contenu que
// l'ancien <racine>/data, recopié une fois par `relocate`). Supprimer ou recompiler le programme ne les touche pas.
// La coquille (jarvis-shell/src/main.rs › data_dir) et le moteur vocal (jarvis-voice/src/util.rs) suivent la même
// règle, recalculée à chaque lancement (pas de variable héritée : un import en attente s'applique à la relance).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::json;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn root() -> PathBuf {
    ROOT.get_or_init(|| {
        if let Ok(r) = std::env::var("JARVIS_ROOT") {
            return PathBuf::from(r);
        }
        // Développement : l'exécutable vit dans <racine>/target/<profil>/ (racine = dossier qui contient ui/ et
        // Cargo.toml). Installé (installer.ps1) : les exécutables sont dans le dossier du programme, avec ui/ et install.json.
        let from_exe = std::env::current_exe().ok().and_then(|exe| {
            exe.ancestors()
                .find(|d| d.join("ui").join("index.html").is_file() && (d.join("Cargo.toml").is_file() || d.join(INSTALL_FILE).is_file()))
                .map(|p| p.to_path_buf())
        });
        from_exe.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(".."))
    })
    .clone()
}

/// Fiche d'une installation (installer.ps1) : version, canal, dépôt de diffusion, modules choisis.
pub const INSTALL_FILE: &str = "install.json";

/// Fiche d'installation, ou None en développement (Jarvis lancé depuis ses sources).
pub fn installed() -> Option<serde_json::Value> {
    let p = root().join(INSTALL_FILE);
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// %LOCALAPPDATA%\Jarvis : données (donnees\), programme installé (app\) et cache de compilation (build\).
pub fn base_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA").ok().filter(|s| !s.trim().is_empty()).map(|d| PathBuf::from(d).join("Jarvis"))
}

/// Ancien emplacement des données, dans le dossier du projet (avant 4.100).
pub fn legacy_data_dir() -> PathBuf {
    root().join("data")
}

/// Emplacement cible des données : %LOCALAPPDATA%\Jarvis\donnees (sinon l'ancien, faute de LOCALAPPDATA).
pub fn target_data_dir() -> PathBuf {
    base_dir().map(|b| b.join("donnees")).unwrap_or_else(legacy_data_dir)
}

/// Données de ce lancement : JARVIS_DATA_DIR s'il est fixé (tests), sinon le dossier cible une fois prêt
/// (VERSION.json), sinon l'ancien <racine>/data tant qu'il n'a pas été recopié : jamais un dossier vide alors que
/// les données sont ailleurs. Même règle dans la coquille et le moteur vocal.
pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("JARVIS_DATA_DIR") {
        return PathBuf::from(d);
    }
    let d = target_data_dir();
    if !d.join(VERSION_FILE).is_file() && legacy_data_dir().join("config.json").is_file() {
        return legacy_data_dir();
    }
    d
}

/// Dossier de travail des agents : <racine>/workspace en développement, %LOCALAPPDATA%\Jarvis\workspace une fois installé
/// (le dossier du programme est remplacé à chaque mise à jour).
pub fn workspace() -> PathBuf {
    if let Ok(w) = std::env::var("JARVIS_WORKSPACE") {
        return PathBuf::from(w);
    }
    match (installed(), base_dir()) {
        (Some(_), Some(b)) => b.join("workspace"),
        _ => root().join("workspace"),
    }
}

/// Dossier des exécutables de Jarvis (jarvis-core.exe, jarvis-voice.exe…).
pub fn bin_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| root().join("target").join("release"))
}

pub fn models_dir() -> PathBuf {
    std::env::var("JARVIS_MODELS_DIR").map(PathBuf::from).unwrap_or_else(|_| data_dir().join("models"))
}

/// Fiche du dossier de données : présente = dossier prêt (créé ou recopié), plus rien à migrer.
pub const VERSION_FILE: &str = "VERSION.json";
/// Format des données ; une version de Jarvis qui ne le connaît pas refuse d'écrire dedans.
pub const DATA_FORMAT: u64 = 1;
/// Jamais recopiés depuis l'ancien dossier : verrous d'instance (propres à un lancement, et celui du nouveau
/// dossier est tenu pendant la copie).
fn skipped(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().ends_with(".lock")
}

fn copy_tree(from: &Path, to: &Path, files: &mut u64, bytes: &mut u64) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if skipped(&e.file_name()) {
            continue;
        }
        if src.is_dir() {
            copy_tree(&src, &dst, files, bytes)?;
        } else {
            let n = std::fs::copy(&src, &dst)?;
            if n != e.metadata()?.len() {
                return Err(std::io::Error::other(format!("copie incomplète : {}", src.display())));
            }
            *files += 1;
            *bytes += n;
        }
    }
    Ok(())
}

/// Ce qu'a fait `relocate`.
#[derive(Debug, PartialEq)]
pub enum Relocated {
    /// Dossier déjà prêt, ou emplacement imposé (JARVIS_DATA_DIR) : rien à faire.
    Nothing,
    /// Aucun ancien dossier : nouveau dossier vide créé.
    Fresh,
    /// Ancien dossier recopié (fichiers, octets), puis renommé en `kept`.
    Copied { files: u64, bytes: u64, kept: Option<PathBuf> },
    /// Import d'une archive appliqué ; le dossier remplacé est gardé en `kept`.
    Imported { kept: Option<PathBuf> },
}

/// Recopie l'ancien dossier `old` (et `old_notes`, les notes rangées à côté) dans `new`, une seule fois :
/// la fiche VERSION.json marque la fin. Copie vérifiée fichier par fichier ; en cas d'échec, `new` est vidé et
/// l'ancien dossier reste intact (Jarvis le relira tant que la copie n'a pas réussi). Après succès, l'ancien dossier
/// est renommé `data.migre-AAAA-MM-JJ` (jamais supprimé) pour qu'aucune donnée ne s'y écrive plus par erreur.
pub fn relocate_dirs(old: &Path, old_notes: &Path, new: &Path) -> std::io::Result<Relocated> {
    if new.join(VERSION_FILE).is_file() {
        return Ok(Relocated::Nothing);
    }
    let stamp = |extra: serde_json::Value| {
        let mut v = json!({ "format": DATA_FORMAT, "jarvis": env!("CARGO_PKG_VERSION"), "creeLe": chrono::Local::now().to_rfc3339() });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        std::fs::write(new.join(VERSION_FILE), serde_json::to_string_pretty(&v).unwrap_or_default())
    };
    std::fs::create_dir_all(new)?;
    let has_old = old != new && old.join("config.json").is_file();
    if !has_old {
        stamp(json!({}))?;
        return Ok(Relocated::Fresh);
    }
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut copy = || -> std::io::Result<()> {
        for e in std::fs::read_dir(old)?.flatten() {
            if skipped(&e.file_name()) {
                continue;
            }
            let (src, dst) = (e.path(), new.join(e.file_name()));
            if src.is_dir() {
                copy_tree(&src, &dst, &mut files, &mut bytes)?;
            } else {
                bytes += std::fs::copy(&src, &dst)?;
                files += 1;
            }
        }
        if old_notes.is_dir() {
            copy_tree(old_notes, &new.join("notes"), &mut files, &mut bytes)?;
        }
        Ok(())
    };
    if let Err(err) = copy() {
        let _ = std::fs::remove_dir_all(new);
        return Err(err);
    }
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let kept = old.with_file_name(format!("data.migre-{day}"));
    let kept = (!kept.exists() && std::fs::rename(old, &kept).is_ok()).then_some(kept);
    stamp(json!({ "migreDepuis": old.to_string_lossy(), "ancienDossier": kept.as_ref().map(|k| k.to_string_lossy().to_string()), "fichiers": files }))?;
    Ok(Relocated::Copied { files, bytes, kept })
}

/// Recopie au premier lancement les données de l'ancien emplacement (<racine>/data, <racine>/notes) vers
/// data_dir(). Rien quand l'emplacement est imposé par JARVIS_DATA_DIR (tests, instances de test).
pub fn relocate() -> std::io::Result<Relocated> {
    if std::env::var("JARVIS_DATA_DIR").is_ok() || base_dir().is_none() {
        return Ok(Relocated::Nothing);
    }
    relocate_dirs(&legacy_data_dir(), &root().join("notes"), &target_data_dir())
}

/// `relocate`, sous le verrou « un seul Jarvis » (aucun autre noyau n'écrit dans l'ancien dossier pendant la copie).
/// Err : migration impossible maintenant ; l'appelant garde alors l'ancien dossier pour ce lancement.
pub fn relocate_guarded() -> std::io::Result<Relocated> {
    if std::env::var("JARVIS_DATA_DIR").is_ok() {
        return Ok(Relocated::Nothing);
    }
    let data = target_data_dir();
    let pending = crate::transfer::pending_dir(&data);
    if data.join(VERSION_FILE).is_file() && !pending.is_dir() {
        return Ok(Relocated::Nothing);
    }
    // Verrou pris à côté des dossiers de données (ni l'ancien ni le nouveau : l'un et l'autre peuvent être renommés).
    let lock_dir = data.parent().map(|p| p.join("verrou")).unwrap_or_else(|| data.clone());
    // Attente courte : à une relance du noyau, le précédent finit de se fermer.
    let lock = crate::instance_lock::InstanceLock::acquire(&lock_dir, true, 10_000, json!({}), &|_| {});
    if lock.is_err() {
        return Err(std::io::Error::other("Jarvis tourne : la migration des données attendra le prochain lancement"));
    }
    let mut res = relocate();
    // Import préparé (transfer.rs) : appliqué maintenant, Jarvis fermé.
    if res.is_ok() && pending.is_dir() {
        res = crate::transfer::apply_pending(&data, &pending).map(|kept| Relocated::Imported { kept });
    }
    drop(lock);
    res
}
/// `jarvis-core donnees-migrer` : lancé par la coquille avant tout le reste (Jarvis fermé).
pub fn relocate_main() -> i32 {
    match relocate_guarded() {
        Ok(Relocated::Copied { files, bytes, kept }) => {
            println!("{files} fichier(s) ({} Mo) recopiés dans {}.", bytes / 1_048_576, target_data_dir().display());
            if let Some(k) = kept {
                println!("Ancien dossier conservé : {}", k.display());
            }
            0
        }
        Ok(_) => 0,
        Err(err) => {
            eprintln!("migration des données impossible : {err}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jarvis-test-relocate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn recopie_une_fois_et_garde_l_ancien_dossier() {
        let t = tmp("copie");
        let (old, notes, new) = (t.join("data"), t.join("notes"), t.join("neuf").join("donnees"));
        std::fs::create_dir_all(old.join("collections").join("images")).unwrap();
        std::fs::create_dir_all(old.join("runtime")).unwrap();
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(old.join("config.json"), "{}").unwrap();
        std::fs::write(old.join("collections").join("images").join("a.jpg"), "img").unwrap();
        std::fs::write(old.join("runtime").join("noyau.lock"), "x").unwrap();
        std::fs::write(old.join("runtime").join("mcp.json"), "{}").unwrap();
        std::fs::write(notes.join("note.md"), "n").unwrap();
        match relocate_dirs(&old, &notes, &new).unwrap() {
            Relocated::Copied { files, kept, .. } => {
                assert_eq!(files, 4);
                assert!(kept.unwrap().join("config.json").is_file());
            }
            other => panic!("{other:?}"),
        }
        assert!(!old.exists());
        assert!(new.join("config.json").is_file() && new.join("collections").join("images").join("a.jpg").is_file());
        assert!(new.join("notes").join("note.md").is_file() && new.join("runtime").join("mcp.json").is_file());
        assert!(!new.join("runtime").join("noyau.lock").exists());
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(new.join(VERSION_FILE)).unwrap()).unwrap();
        assert_eq!(v["format"], DATA_FORMAT);
        // Seconde fois : rien (idempotent), même si un ancien dossier réapparaît.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.json"), "{\"autre\":1}").unwrap();
        assert_eq!(relocate_dirs(&old, &notes, &new).unwrap(), Relocated::Nothing);
        assert_eq!(std::fs::read_to_string(new.join("config.json")).unwrap(), "{}");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn sans_ancien_dossier_cree_un_dossier_vierge() {
        let t = tmp("vierge");
        let new = t.join("donnees");
        assert_eq!(relocate_dirs(&t.join("data"), &t.join("notes"), &new).unwrap(), Relocated::Fresh);
        assert!(new.join(VERSION_FILE).is_file());
        let _ = std::fs::remove_dir_all(&t);
    }
}
