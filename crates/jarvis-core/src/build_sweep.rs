// Place occupée par les compilations (dossier target/) et effacement des anciennes copies des crates.
//
// La version de Jarvis entre dans l'empreinte que Cargo donne à chaque crate du dépôt : à chaque nouvelle
// version, jarvis-core, jarvis-voice… sont recompilées sous un autre nom et les copies précédentes restent
// (plusieurs centaines de Mo chacune). Seules les crates du dépôt (« jarvis… ») sont visées, jamais les
// dépendances externes : une copie est ancienne quand une plus récente de la même crate existe et qu'elle
// date de plus d'une heure avant celle-ci (une compilation et ses tests tiennent dans cette fenêtre). La
// copie la plus récente de chaque sorte (bibliothèque, exécutable, vérification) est toujours gardée.
// Supprimer trop n'est jamais grave : Cargo recompile ce qui manque.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::json;

use crate::js::J;

/// Écart toléré avec la copie la plus récente d'une crate.
const KEEP_WINDOW: Duration = Duration::from_secs(3600);

pub fn target_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|_| crate::paths::root().join("target"))
}

/// Taille d'un fichier ou d'un dossier (récursif).
pub fn size_of(p: &Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else { return 0 };
    if !m.is_dir() {
        return m.len();
    }
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0)
}

fn mtime(p: &Path) -> SystemTime {
    std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Date d'un dossier : celle de son fichier le plus récent sur deux niveaux (incremental/<crate>/<session>/…),
/// sinon celle du dossier.
fn dir_time(p: &Path) -> SystemTime {
    fn newest(p: &Path, depth: u8) -> Option<SystemTime> {
        let mut best = None;
        for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
            let path = e.path();
            let t = if path.is_dir() { if depth > 0 { newest(&path, depth - 1) } else { None } } else { Some(mtime(&path)) };
            best = best.max(t);
        }
        best
    }
    newest(p, 1).unwrap_or_else(|| mtime(p))
}

fn is_hash16(s: &str) -> bool {
    s.len() == 16 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// « libjarvis_core-90492853d3e5c470.rlib » → (« jarvis_core », « 90492853d3e5c470 ») ; None hors crates du dépôt.
fn deps_name(name: &str) -> Option<(String, String)> {
    let base = name.split('.').next()?;
    let (stem, hash) = base.rsplit_once('-')?;
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    (is_hash16(hash) && stem.starts_with("jarvis")).then(|| (stem.to_string(), hash.to_string()))
}

/// Une copie d'une crate : ses fichiers ou son dossier, sa sorte et sa date.
#[derive(Debug)]
struct Unit {
    stem: String,
    kind: &'static str,
    time: SystemTime,
    paths: Vec<PathBuf>,
    hash: Option<String>,
}

fn deps_units(deps: &Path) -> Vec<Unit> {
    let mut groups: HashMap<(String, String), Vec<PathBuf>> = HashMap::new();
    for e in std::fs::read_dir(deps).into_iter().flatten().flatten() {
        if let Some(key) = deps_name(&e.file_name().to_string_lossy()) {
            groups.entry(key).or_default().push(e.path());
        }
    }
    groups
        .into_iter()
        .map(|((stem, hash), paths)| {
            let has = |ext: &str| paths.iter().any(|p| p.extension().is_some_and(|x| x == ext));
            let kind = if has("exe") { "exe" } else if has("rlib") { "rlib" } else if has("dll") { "dll" } else { "rmeta" };
            let time = paths.iter().map(|p| mtime(p)).max().unwrap_or(SystemTime::UNIX_EPOCH);
            Unit { stem, kind, time, paths, hash: Some(hash) }
        })
        .collect()
}

/// Dossiers « <crate>-<suffixe> » de incremental/ ou build/.
fn dir_units(dir: &Path, build: bool) -> Vec<Unit> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let Some((stem, suffix)) = name.rsplit_once('-') else { continue };
        if !p.is_dir() || !stem.starts_with("jarvis") || (build && !is_hash16(suffix)) {
            continue;
        }
        // build/ : le script compilé et le résultat de son exécution sont deux sortes distinctes.
        let kind = if !build {
            "incremental"
        } else if std::fs::read_dir(&p).into_iter().flatten().flatten().any(|f| f.file_name().to_string_lossy().starts_with("build-script-build")) {
            "script"
        } else {
            "out"
        };
        out.push(Unit { stem: stem.to_string(), kind, time: dir_time(&p), paths: vec![p], hash: None });
    }
    out
}

/// Copies à effacer parmi celles d'un même dossier.
fn stale(units: Vec<Unit>) -> Vec<Unit> {
    let mut newest: HashMap<String, SystemTime> = HashMap::new();
    let mut newest_kind: HashMap<(String, &'static str), SystemTime> = HashMap::new();
    for u in &units {
        let n = newest.entry(u.stem.clone()).or_insert(u.time);
        *n = (*n).max(u.time);
        let k = newest_kind.entry((u.stem.clone(), u.kind)).or_insert(u.time);
        *k = (*k).max(u.time);
    }
    units
        .into_iter()
        .filter(|u| u.time + KEEP_WINDOW < newest[&u.stem] && u.time < newest_kind[&(u.stem.clone(), u.kind)])
        .collect()
}

/// Profils de compilation présents (debug, release…) : sous-dossiers de target/ qui ont un deps/.
fn profiles(target: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(target).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.join("deps").is_dir()).collect();
    out.sort();
    out
}

/// Anciennes copies d'un profil : chemins à effacer et empreintes Cargo correspondantes.
fn stale_paths(profile: &Path) -> Vec<PathBuf> {
    let mut paths = vec![];
    let mut hashes = HashSet::new();
    for u in stale(deps_units(&profile.join("deps"))) {
        hashes.extend(u.hash);
        paths.extend(u.paths);
    }
    paths.extend(stale(dir_units(&profile.join("incremental"), false)).into_iter().flat_map(|u| u.paths));
    paths.extend(stale(dir_units(&profile.join("build"), true)).into_iter().flat_map(|u| u.paths));
    // .fingerprint/<crate>-<empreinte> des copies effacées : Cargo ne les retrouvera plus.
    for e in std::fs::read_dir(profile.join(".fingerprint")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("jarvis") && name.rsplit_once('-').is_some_and(|(_, h)| hashes.contains(h)) {
            paths.push(e.path());
        }
    }
    paths
}

/// Mesure : { ok, dossier, octets, profils: [{ nom, octets, anciennes }], anciennes, copies, mesureLe }.
pub fn scan(target: &Path) -> J {
    if !target.is_dir() {
        return json!({ "ok": true, "dossier": target.display().to_string(), "octets": 0, "profils": [], "anciennes": 0, "copies": 0, "mesureLe": crate::js::now_ms() });
    }
    let (mut stale_bytes, mut copies) = (0u64, 0usize);
    let mut rows = vec![];
    for p in profiles(target) {
        let old = stale_paths(&p);
        let old_bytes: u64 = old.iter().map(|x| size_of(x)).sum();
        stale_bytes += old_bytes;
        copies += old.len();
        rows.push(json!({ "nom": p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), "octets": size_of(&p), "anciennes": old_bytes }));
    }
    json!({
        "ok": true, "dossier": target.display().to_string(), "octets": size_of(target), "profils": rows,
        "anciennes": stale_bytes, "copies": copies, "mesureLe": crate::js::now_ms()
    })
}

/// Efface les anciennes copies de tous les profils. Un fichier verrouillé (exécutable lancé) est passé.
pub fn sweep(target: &Path) -> J {
    let (mut freed, mut removed, mut failed) = (0u64, 0usize, 0usize);
    for p in profiles(target) {
        for path in stale_paths(&p) {
            let n = size_of(&path);
            let r = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            match r {
                Ok(_) => {
                    freed += n;
                    removed += 1;
                }
                // Dossier en partie effacé : on compte ce qui est vraiment parti.
                Err(_) => {
                    failed += 1;
                    freed += n.saturating_sub(size_of(&path));
                }
            }
        }
    }
    json!({ "ok": true, "liberes": freed, "effaces": removed, "echecs": failed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File, FileTimes};

    fn touch(p: &Path, age_s: u64, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; bytes]).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_s);
        File::options().write(true).open(p).unwrap().set_times(FileTimes::new().set_modified(t)).unwrap();
    }

    #[test]
    fn noms_des_copies() {
        assert_eq!(deps_name("libjarvis_core-90492853d3e5c470.rlib"), Some(("jarvis_core".into(), "90492853d3e5c470".into())));
        assert_eq!(deps_name("jarvis_core-90492853d3e5c470.12abc.rcgu.o"), Some(("jarvis_core".into(), "90492853d3e5c470".into())));
        assert_eq!(deps_name("libserde-90492853d3e5c470.rlib"), None);
        assert_eq!(deps_name("jarvis_core.d"), None);
    }

    #[test]
    fn efface_les_anciennes_copies_seulement() {
        let t = std::env::temp_dir().join(format!("jarvis-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        let deps = t.join("debug").join("deps");
        // Version courante (bibliothèque + tests, compilés ensemble) et deux anciennes versions.
        touch(&deps.join("libjarvis_core-aaaaaaaaaaaaaaaa.rlib"), 0, 100);
        touch(&deps.join("jarvis_core-bbbbbbbbbbbbbbbb.exe"), 600, 100);
        touch(&deps.join("libjarvis_core-cccccccccccccccc.rlib"), 90_000, 100);
        touch(&deps.join("libjarvis_core-cccccccccccccccc.rmeta"), 90_000, 10);
        touch(&deps.join("jarvis_core-dddddddddddddddd.exe"), 90_000, 100);
        // Seule vérification connue (cargo check ancien) : la plus récente de sa sorte, gardée.
        touch(&deps.join("libjarvis_core-eeeeeeeeeeeeeeee.rmeta"), 200_000, 10);
        // Dépendance externe ancienne : jamais touchée.
        touch(&deps.join("libserde-ffffffffffffffff.rlib"), 900_000, 100);
        touch(&t.join("debug").join(".fingerprint").join("jarvis-core-cccccccccccccccc").join("lib-jarvis_core"), 90_000, 1);
        touch(&t.join("debug").join("incremental").join("jarvis_core-1abc").join("s-1").join("f"), 0, 50);
        touch(&t.join("debug").join("incremental").join("jarvis_core-2old").join("s-1").join("f"), 90_000, 50);
        let old_inc = t.join("debug").join("incremental").join("jarvis_core-2old");

        let r = scan(&t);
        assert_eq!(r["copies"], json!(5), "{r}");
        assert_eq!(r["anciennes"], json!(100 + 10 + 100 + 1 + 50));
        let s = sweep(&t);
        assert_eq!(s["effaces"], json!(5), "{s}");
        assert!(deps.join("libjarvis_core-aaaaaaaaaaaaaaaa.rlib").exists());
        assert!(deps.join("jarvis_core-bbbbbbbbbbbbbbbb.exe").exists());
        assert!(deps.join("libjarvis_core-eeeeeeeeeeeeeeee.rmeta").exists());
        assert!(deps.join("libserde-ffffffffffffffff.rlib").exists());
        assert!(!deps.join("libjarvis_core-cccccccccccccccc.rlib").exists());
        assert!(!deps.join("jarvis_core-dddddddddddddddd.exe").exists());
        assert!(!t.join("debug").join(".fingerprint").join("jarvis-core-cccccccccccccccc").exists());
        assert!(!old_inc.exists());
        assert!(t.join("debug").join("incremental").join("jarvis_core-1abc").exists());
        assert_eq!(scan(&t)["copies"], json!(0));
        let _ = fs::remove_dir_all(&t);
    }

    /// Essai réel, mesure seule (rien n'est effacé) : JARVIS_TEST_SWEEP=1 cargo test -p jarvis-core build_sweep -- --ignored --nocapture
    #[test]
    #[ignore]
    fn mesure_reelle() {
        if std::env::var("JARVIS_TEST_SWEEP").is_err() {
            return;
        }
        let t0 = std::time::Instant::now();
        println!("{}  ({:?})", scan(&target_dir()), t0.elapsed());
    }
}
