// Reprise des données de Jarvis v3 (tools/migrate-from-v1.js de la v2, adapté) :
//
//   jarvis-core migrate --from "E:\www\A-En cours\jarvis-v3" [--force]
//
// La v3 n'est que lue. Copiés dans le dossier de données de la v4 : réglages, mémoire (et ses
// vecteurs), journal scellé (la chaîne continue telle quelle : les empreintes sont calculées à
// l'identique), listes, échecs, émotions, connecteurs, empreintes vocales, corpus vocal, réglages
// du widget et du Bureau, notes et réunions archivées. Les secrets chiffrés par Windows (DPAPI,
// utilisateur courant) restent lisibles. Rapport : data/migration-v3.json, inscrit au journal au
// premier démarrage (entrée « migration_v3 »).

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::js;

const SKIP: &[&str] = &["logs", "runtime", "models", "migration-v1.json"];

fn copy_dir(src: &Path, dst: &Path, copied: &mut Vec<String>, rel: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let (s, d) = (e.path(), dst.join(&name));
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if s.is_dir() {
            copy_dir(&s, &d, copied, &r)?;
        } else {
            std::fs::copy(&s, &d)?;
            copied.push(r);
        }
    }
    Ok(())
}

/// Dernière entrée du journal scellé : (nombre d'entrées, empreinte finale).
fn journal_tail(file: &Path) -> Option<(i64, String)> {
    let text = std::fs::read_to_string(file).ok()?;
    let last = text.lines().rev().find(|l| !l.trim().is_empty())?;
    let v: serde_json::Value = serde_json::from_str(last).ok()?;
    Some((v["seq"].as_i64().unwrap_or(0), js::gs(&v, "hash")))
}

pub fn main(args: &[String]) -> i32 {
    let from = args.iter().position(|a| a == "--from").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let force = args.iter().any(|a| a == "--force");
    let Some(v3) = from else {
        eprintln!("usage : jarvis-core migrate --from <dossier de Jarvis v3> [--force]");
        return 1;
    };
    let src = v3.join("data");
    if !src.join("memory.json").is_file() && !src.join("config.json").is_file() {
        eprintln!("{} ne ressemble pas à un dossier de données de Jarvis v3", src.display());
        return 1;
    }
    let dst = crate::paths::data_dir();
    if !force && (dst.join("memory.json").is_file() || dst.join("history").join("journal.jsonl").is_file()) {
        eprintln!("{} contient déjà des données de la v4 : rien n'est écrasé (ajouter --force pour remplacer)", dst.display());
        return 2;
    }
    let _ = std::fs::create_dir_all(&dst);
    let mut copied = vec![];
    let mut errors = vec![];
    for e in std::fs::read_dir(&src).map(|r| r.flatten().collect::<Vec<_>>()).unwrap_or_default() {
        let name = e.file_name().to_string_lossy().to_string();
        if SKIP.contains(&name.as_str()) || name.starts_with("voiceprint.avant") {
            continue;
        }
        let res = if e.path().is_dir() { copy_dir(&e.path(), &dst.join(&name), &mut copied, &name) } else { std::fs::copy(e.path(), dst.join(&name)).map(|_| copied.push(name.clone())) };
        if let Err(err) = res {
            errors.push(format!("{name} : {err}"));
        }
    }
    // Modèles locaux déjà téléchargés par la v3 (mot d'éveil, empreinte, voix Piper).
    for m in ["kws", "speaker", "tts"] {
        let (s, d) = (src.join("models").join(m), crate::paths::models_dir().join(m));
        if s.is_dir() && !d.exists() {
            if let Err(err) = copy_dir(&s, &d, &mut copied, &format!("models/{m}")) {
                errors.push(format!("models/{m} : {err}"));
            }
        }
    }
    // Notes de la v3 (dossier notes/ à la racine du projet).
    let notes = v3.join("notes");
    if notes.is_dir() {
        if let Err(err) = copy_dir(&notes, &crate::paths::data_dir().join("notes"), &mut copied, "notes") {
            errors.push(format!("notes : {err}"));
        }
    }
    let journal = journal_tail(&src.join("history").join("journal.jsonl")).map(|(count, hash)| json!({ "count": count, "hash": hash }));
    let report = json!({
        "from": v3.display().to_string(),
        "at": js::iso_now(),
        "copied": copied.len(),
        "files": copied,
        "errors": errors,
        "v3Journal": journal,
        "secretsToReenter": []
    });
    let _ = js::write_json(&dst.join("migration-v3.json"), &report);
    println!(
        "migration terminée : {} fichier(s) copiés depuis {}{}",
        report["copied"],
        v3.display(),
        if errors.is_empty() { String::new() } else { format!(", {} erreur(s)", errors.len()) }
    );
    if let Some(j) = &report["v3Journal"].as_object() {
        println!("journal scellé repris : {} entrées, empreinte finale {}", j["count"], j["hash"]);
    }
    if errors.is_empty() {
        0
    } else {
        for e in &errors {
            eprintln!("  {e}");
        }
        3
    }
}
