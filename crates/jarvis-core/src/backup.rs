// Sauvegarde automatique de data/ : mémoire, savoir (RAG), empreinte vocale, réglages, journal scellé, listes, corpus.
//
// Une copie par jour dans un dossier hors du projet (par défaut Documents\Jarvis-sauvegardes\AAAA-MM-JJ),
// refaite toutes les 6 h tant que Jarvis tourne. On garde les 7 jours les plus récents, plus une copie
// par semaine sur les 4 semaines suivantes. Les modèles (plusieurs Go), les journaux de diagnostic et le
// cache vocal ne sont pas copiés. Les secrets des connecteurs sont déjà chiffrés par Windows (DPAPI)
// dans les fichiers : la copie les garde tels quels.
//
//   jarvis-core restaurer [--depuis <dossier>] --oui    (Jarvis fermé) remet la sauvegarde la plus récente

use std::path::{Path, PathBuf};
use std::rc::Rc;

use chrono::{Datelike, NaiveDate};
use serde_json::json;

use crate::js::{self, Timer, J};

const EVERY_MS: u64 = 6 * 3600 * 1000;
const DAILY_KEPT: usize = 7;
const WEEKLY_KEPT: usize = 4;
const MARKER: &str = ".sauvegarde.json";
/// Dossiers techniques de data/ jamais copiés (modèles, journaux, état d'exécution, cache vocal recalculable) ;
/// tous les autres dossiers, et les fichiers .json de la racine, sont des données de l'utilisateur.
pub const SKIP_DIRS: &[&str] = &["models", "logs", "runtime", "voice-cache"];

pub struct Backup {
    data: PathBuf,
    config: Box<dyn Fn() -> J>,
    record: Rc<dyn Fn(J)>,
    log: Rc<dyn Fn(String)>,
    timer: Timer,
}

/// Dossier de destination : réglage « backup.dir », sinon Documents\Jarvis-sauvegardes.
pub fn destination(cfg: &J) -> PathBuf {
    let dir = js::gs(cfg, "dir");
    if !dir.trim().is_empty() {
        return PathBuf::from(dir.trim());
    }
    let home = js::env("USERPROFILE").or_else(|| js::env("HOME")).unwrap_or_else(|| "C:\\Users\\Default".into());
    PathBuf::from(home).join("Documents").join("Jarvis-sauvegardes")
}

fn copy_dir(from: &Path, to: &Path, files: &mut u32, bytes: &mut u64) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst, files, bytes)?;
        } else if let Ok(n) = std::fs::copy(&src, &dst) {
            *files += 1;
            *bytes += n;
        }
    }
    Ok(())
}

/// Copie data/ dans `dest/<jour>`. Retourne (dossier, fichiers, octets).
pub fn run(data: &Path, dest: &Path, day: &str) -> std::io::Result<(PathBuf, u32, u64)> {
    let target = dest.join(day);
    std::fs::create_dir_all(&target)?;
    let (mut files, mut bytes) = (0u32, 0u64);
    for e in std::fs::read_dir(data)?.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_file() && name.ends_with(".json") {
            if let Ok(n) = std::fs::copy(&p, target.join(&name)) {
                files += 1;
                bytes += n;
            }
        }
    }
    for e in std::fs::read_dir(data)?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if e.path().is_dir() && !SKIP_DIRS.contains(&name.as_str()) {
            copy_dir(&e.path(), &target.join(&name), &mut files, &mut bytes)?;
        }
    }
    let _ = js::write_json(&target.join(MARKER), &json!({ "at": js::now_ms(), "fichiers": files, "octets": bytes }));
    Ok((target, files, bytes))
}

/// Jours de sauvegarde présents (AAAA-MM-JJ), du plus récent au plus ancien.
pub fn days(dest: &Path) -> Vec<(NaiveDate, PathBuf)> {
    let mut out: Vec<(NaiveDate, PathBuf)> = std::fs::read_dir(dest)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let d = NaiveDate::parse_from_str(&e.file_name().to_string_lossy(), "%Y-%m-%d").ok()?;
                    e.path().is_dir().then(|| (d, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

/// Jours à supprimer : on garde les `DAILY_KEPT` plus récents et, parmi les autres, le plus récent de
/// chacune des `WEEKLY_KEPT` semaines les plus récentes.
pub fn to_prune(sorted_desc: &[NaiveDate]) -> Vec<NaiveDate> {
    let mut keep: Vec<NaiveDate> = sorted_desc.iter().take(DAILY_KEPT).copied().collect();
    let mut weeks: Vec<(i32, u32)> = vec![];
    for d in sorted_desc.iter().skip(DAILY_KEPT) {
        let w = (d.iso_week().year(), d.iso_week().week());
        if !weeks.contains(&w) && weeks.len() < WEEKLY_KEPT {
            weeks.push(w);
            keep.push(*d);
        }
    }
    sorted_desc.iter().filter(|d| !keep.contains(d)).copied().collect()
}

impl Backup {
    pub fn new(data: PathBuf, config: Box<dyn Fn() -> J>, record: Rc<dyn Fn(J)>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        Rc::new(Self { data, config, record, log, timer: Timer::new() })
    }

    pub fn start(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        // Première vérification peu après le démarrage (le noyau a d'abord à faire), puis toutes les 6 h.
        js::set_timeout(90_000, {
            let me = me.clone();
            move || {
                if let Some(b) = me.upgrade() {
                    b.tick();
                }
            }
        });
        self.timer.every(EVERY_MS, move || {
            if let Some(b) = me.upgrade() {
                b.tick();
            }
        });
    }

    fn tick(&self) {
        let cfg = (self.config)();
        if cfg["enabled"] == json!(false) {
            return;
        }
        let dest = destination(&cfg);
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        // Déjà faite il y a moins de 6 h : rien à refaire.
        let last = js::read_json(&dest.join(&day).join(MARKER)).and_then(|m| m["at"].as_i64()).unwrap_or(0);
        if js::now_ms() - last < EVERY_MS as i64 - 60_000 {
            return;
        }
        match run(&self.data, &dest, &day) {
            Ok((dir, files, bytes)) => {
                (self.log)(format!("[sauvegarde] {files} fichier(s), {} Ko → {}", bytes / 1024, dir.display()));
                (self.record)(json!({ "dossier": dir.to_string_lossy(), "fichiers": files, "octets": bytes }));
                let all = days(&dest);
                let dates: Vec<NaiveDate> = all.iter().map(|(d, _)| *d).collect();
                for old in to_prune(&dates) {
                    if let Some((_, p)) = all.iter().find(|(d, _)| *d == old) {
                        let _ = std::fs::remove_dir_all(p);
                    }
                }
            }
            Err(e) => (self.log)(format!("[sauvegarde] échec : {e}")),
        }
    }
}

/// `jarvis-core restaurer [--depuis <dossier>] --oui`
pub fn main(args: &[String]) -> i32 {
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let data = crate::paths::data_dir();
    let cfg = js::read_json(&data.join("config.json")).map(|c| c["backup"].clone()).unwrap_or(J::Null);
    let from = match arg("--depuis") {
        Some(d) => PathBuf::from(d),
        None => match days(&destination(&cfg)).into_iter().next() {
            Some((_, p)) => p,
            None => {
                eprintln!("aucune sauvegarde dans {}", destination(&cfg).display());
                return 1;
            }
        },
    };
    println!("Sauvegarde : {}", from.display());
    println!("Destination : {}", data.display());
    // Le verrou du noyau n'est pas libre : Jarvis tourne.
    if crate::instance_lock::InstanceLock::acquire(&data, false, 0, json!({}), &|_| {}).is_err() {
        eprintln!("Jarvis tourne : ferme-le d'abord.");
        return 1;
    }
    if !args.iter().any(|a| a == "--oui") {
        println!("Rien n'a été modifié. Ajoute --oui pour remplacer les fichiers de data/ par ceux de la sauvegarde.");
        return 0;
    }
    let (mut files, mut bytes) = (0u32, 0u64);
    for e in std::fs::read_dir(&from).into_iter().flatten().flatten() {
        let (src, name) = (e.path(), e.file_name().to_string_lossy().to_string());
        if name == MARKER {
            continue;
        }
        let dst = data.join(&name);
        if src.is_dir() {
            if let Err(err) = copy_dir(&src, &dst, &mut files, &mut bytes) {
                eprintln!("échec sur {name} : {err}");
                return 1;
            }
        } else if let Ok(n) = std::fs::copy(&src, &dst) {
            files += 1;
            bytes += n;
        }
    }
    println!("{files} fichier(s) restauré(s) ({} Ko).", bytes / 1024);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn garde_sept_jours_puis_une_copie_par_semaine() {
        // 20 jours consécutifs, du plus récent (2026-10-16) au plus ancien.
        let mut days: Vec<NaiveDate> = (0..20).map(|i| d("2026-10-16") - chrono::Duration::days(i)).collect();
        days.sort_by(|a, b| b.cmp(a));
        let pruned = to_prune(&days);
        let kept: Vec<_> = days.iter().filter(|x| !pruned.contains(x)).collect();
        // 7 jours + au plus 4 semaines distinctes.
        assert!(kept.len() >= 8 && kept.len() <= 11, "{}", kept.len());
        assert!(days.iter().take(7).all(|x| !pruned.contains(x)));
        // Au-delà des 7 plus récents, une seule copie par semaine ISO.
        let mut weeks: Vec<_> = kept.iter().skip(7).map(|x| (x.iso_week().year(), x.iso_week().week())).collect();
        let n = weeks.len();
        weeks.dedup();
        assert_eq!(weeks.len(), n);
    }

    #[test]
    fn copie_les_json_et_les_dossiers_sans_les_modeles() {
        let tmp = std::env::temp_dir().join(format!("jarvis-test-sauvegarde-{}", std::process::id()));
        let (data, dest) = (tmp.join("data"), tmp.join("sauv"));
        std::fs::create_dir_all(data.join("history")).unwrap();
        std::fs::create_dir_all(data.join("models")).unwrap();
        // Dossiers de données qu'une liste fixe oubliait (listes à fiches, savoir RAG) : copiés d'office.
        std::fs::create_dir_all(data.join("collections")).unwrap();
        std::fs::create_dir_all(data.join("savoir")).unwrap();
        std::fs::create_dir_all(data.join("logs")).unwrap();
        std::fs::write(data.join("memory.json"), "{}").unwrap();
        std::fs::write(data.join("history").join("journal.jsonl"), "x").unwrap();
        std::fs::write(data.join("collections").join("vinyles.json"), "x").unwrap();
        std::fs::write(data.join("savoir").join("index.json"), "x").unwrap();
        std::fs::write(data.join("models").join("gros.onnx"), "x").unwrap();
        std::fs::write(data.join("logs").join("core.log"), "x").unwrap();
        let (dir, files, _) = run(&data, &dest, "2026-10-16").unwrap();
        assert_eq!(files, 4);
        assert!(dir.join("memory.json").is_file() && dir.join("history").join("journal.jsonl").is_file());
        assert!(dir.join("collections").join("vinyles.json").is_file() && dir.join("savoir").join("index.json").is_file());
        assert!(!dir.join("models").exists() && !dir.join("logs").exists());
        assert_eq!(days(&dest).len(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
