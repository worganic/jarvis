// Transfert des données de l'utilisateur : une archive « .jarvis » (zip) pour passer sur un autre PC, un autre compte
// Windows ou une installation neuve.
//
// Contenu : tout le dossier de données sauf l'état technique (modèles, journaux, état d'exécution, cache vocal :
// backup.rs › SKIP_DIRS), plus manifeste.json (format, version, empreinte SHA-256 de chaque fichier).
// Les secrets des connecteurs sont chiffrés par Windows (DPAPI) : illisibles ailleurs. À l'export ils sont repérés à
// leur en-tête DPAPI, déchiffrés puis rechiffrés par un mot de passe (Argon2id + XChaCha20-Poly1305) ; sans mot de passe,
// ils sont retirés (à ressaisir). À l'import ils repassent en DPAPI pour le compte courant.
//
// L'import ne touche jamais les données d'un Jarvis qui tourne : l'archive est vérifiée puis préparée dans
// import-en-attente/, appliquée au lancement suivant (paths.rs › relocate_guarded) ; le dossier remplacé est gardé
// (donnees.avant-import-AAAA-MM-JJ-HHMM) et les modèles y sont repris, sans nouveau téléchargement.
//
//   jarvis-core donnees exporter <fichier.jarvis> [--mot-de-passe <m>]
//   jarvis-core donnees apercu <fichier.jarvis> [--mot-de-passe <m>]
//   jarvis-core donnees importer <fichier.jarvis> [--mot-de-passe <m>] --oui     (Jarvis fermé : appliqué aussitôt)

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde_json::{json, Value as J};
use sha2::{Digest, Sha256};

use crate::secret_box::SecretBox;

pub const EXTENSION: &str = "jarvis";
const MANIFEST: &str = "manifeste.json";
const PREFIX: &str = "donnees/";
/// En-tête d'un bloc DPAPI en base64 (version 1 + identifiant du fournisseur).
const DPAPI_B64: &str = "AQAAANCMnd8BFdERjHoAwE/Cl+s";
const SEALED: &str = "jarvis-secret:v1:";
const CHECK_TEXT: &str = "jarvis";
/// Dossier voisin du dossier de données où un import attend le prochain lancement.
pub const PENDING: &str = "import-en-attente";

type Res<T> = Result<T, String>;

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

// --- Chiffrement par mot de passe ------------------------------------------------------------------------------------

struct Sealer(XChaCha20Poly1305);

impl Sealer {
    fn new(password: &str, salt: &[u8]) -> Res<Self> {
        let mut key = [0u8; 32];
        argon2::Argon2::default().hash_password_into(password.as_bytes(), salt, &mut key).map_err(|e| e.to_string())?;
        Ok(Self(XChaCha20Poly1305::new((&key).into())))
    }

    fn seal(&self, clear: &str) -> Res<String> {
        let mut nonce = [0u8; 24];
        getrandom::getrandom(&mut nonce).map_err(|e| e.to_string())?;
        let ct = self.0.encrypt(XNonce::from_slice(&nonce), clear.as_bytes()).map_err(|_| "chiffrement impossible".to_string())?;
        Ok(format!("{SEALED}{}", b64().encode([nonce.as_slice(), &ct].concat())))
    }

    fn open(&self, sealed: &str) -> Res<String> {
        let raw = b64().decode(sealed.trim_start_matches(SEALED)).map_err(|e| e.to_string())?;
        if raw.len() < 24 {
            return Err("secret illisible".into());
        }
        let clear = self.0.decrypt(XNonce::from_slice(&raw[..24]), &raw[24..]).map_err(|_| "mot de passe incorrect".to_string())?;
        String::from_utf8(clear).map_err(|e| e.to_string())
    }
}

/// Remplace chaque chaîne de `v` pour laquelle `f` rend Some. Retourne le nombre de remplacements.
fn map_strings(v: &mut J, f: &mut dyn FnMut(&str) -> Res<Option<String>>) -> Res<u32> {
    let mut n = 0;
    match v {
        J::String(s) => {
            if let Some(r) = f(s)? {
                *s = r;
                n += 1;
            }
        }
        J::Array(a) => {
            for x in a {
                n += map_strings(x, f)?;
            }
        }
        J::Object(o) => {
            for (_, x) in o.iter_mut() {
                n += map_strings(x, f)?;
            }
        }
        _ => {}
    }
    Ok(n)
}

// --- Export ------------------------------------------------------------------------------------------------------------

fn skipped(rel: &str) -> bool {
    let first = rel.split('/').next().unwrap_or("");
    crate::backup::SKIP_DIRS.contains(&first) || rel.ends_with(".lock") || first == PENDING
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
        if skipped(&rel) {
            continue;
        }
        if p.is_dir() {
            walk(&p, base, out);
        } else {
            out.push((rel, p));
        }
    }
}

#[derive(Debug)]
pub struct ExportReport {
    pub files: u64,
    pub bytes: u64,
    /// Secrets rechiffrés par le mot de passe (ou retirés, sans mot de passe).
    pub secrets: u32,
    pub sealed: bool,
}

/// Écrit l'archive `out` à partir du dossier de données `data`. `unprotect` déchiffre un bloc DPAPI.
pub fn export(data: &Path, out: &Path, password: Option<&str>, unprotect: &dyn Fn(&str) -> Res<String>) -> Res<ExportReport> {
    let password = password.filter(|p| !p.is_empty());
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).map_err(|e| e.to_string())?;
    let sealer = password.map(|p| Sealer::new(p, &salt)).transpose()?;
    let mut files = vec![];
    walk(data, data, &mut files);
    files.sort();
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = out.with_extension("partiel");
    let file = std::fs::File::create(&tmp).map_err(|e| format!("{} : {e}", tmp.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    let (mut listed, mut bytes, mut secrets, mut without) = (vec![], 0u64, 0u32, vec![]);
    for (rel, path) in &files {
        let mut content = std::fs::read(path).map_err(|e| format!("{rel} : {e}"))?;
        // Secrets DPAPI d'un fichier JSON : rechiffrés (mot de passe) ou retirés.
        if rel.ends_with(".json") && String::from_utf8_lossy(&content).contains(DPAPI_B64) {
            if let Ok(mut v) = serde_json::from_slice::<J>(&content) {
                let n = map_strings(&mut v, &mut |s| {
                    if !s.starts_with(DPAPI_B64) {
                        return Ok(None);
                    }
                    match &sealer {
                        Some(k) => unprotect(s).and_then(|clear| k.seal(&clear)).map(Some),
                        None => Ok(Some(String::new())),
                    }
                })?;
                if n > 0 {
                    secrets += n;
                    if sealer.is_none() {
                        without.push(rel.clone());
                    }
                    content = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
                }
            }
        }
        zip.start_file(format!("{PREFIX}{rel}"), opts).map_err(|e| e.to_string())?;
        zip.write_all(&content).map_err(|e| e.to_string())?;
        bytes += content.len() as u64;
        listed.push(json!({ "chemin": rel, "octets": content.len(), "sha256": sha(&content) }));
    }
    let manifest = json!({
        "format": crate::paths::DATA_FORMAT,
        "jarvis": env!("CARGO_PKG_VERSION"),
        "creeLe": chrono::Local::now().to_rfc3339(),
        "secrets": if sealer.is_some() { "chiffres" } else { "retires" },
        "secretsRetiresDe": without,
        "sel": b64().encode(salt),
        "verif": sealer.as_ref().map(|k| k.seal(CHECK_TEXT)).transpose()?,
        "fichiers": listed,
    });
    zip.start_file(MANIFEST, opts).map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string_pretty(&manifest).unwrap_or_default().as_bytes()).map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())?;
    Ok(ExportReport { files: files.len() as u64, bytes, secrets, sealed: sealer.is_some() })
}

// --- Lecture et vérification ---------------------------------------------------------------------------------------------

pub struct Archive {
    pub manifest: J,
    /// (chemin relatif, contenu), vérifiés contre le manifeste.
    pub files: Vec<(String, Vec<u8>)>,
    sealer: Option<Sealer>,
}

impl Archive {
    /// Résumé lisible : version d'origine, date, nombre de fichiers par dossier de premier niveau, secrets.
    pub fn preview(&self) -> J {
        let mut groups: std::collections::BTreeMap<String, (u64, u64)> = Default::default();
        for (rel, c) in &self.files {
            let g = if rel.contains('/') { rel.split('/').next().unwrap_or("").to_string() } else { "réglages et mémoire (.json)".into() };
            let e = groups.entry(g).or_default();
            e.0 += 1;
            e.1 += c.len() as u64;
        }
        json!({
            "jarvis": self.manifest["jarvis"],
            "creeLe": self.manifest["creeLe"],
            "fichiers": self.files.len(),
            "octets": self.files.iter().map(|(_, c)| c.len() as u64).sum::<u64>(),
            "secrets": self.manifest["secrets"],
            "secretsRetiresDe": self.manifest["secretsRetiresDe"],
            "dossiers": groups.into_iter().map(|(k, (n, b))| json!({ "nom": k, "fichiers": n, "octets": b })).collect::<Vec<_>>(),
        })
    }
}

/// Ouvre et vérifie l'archive : format connu, aucun chemin hors du dossier, empreintes conformes, mot de passe juste.
pub fn open(path: &Path, password: Option<&str>) -> Res<Archive> {
    let file = std::fs::File::open(path).map_err(|e| format!("{} : {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "ce fichier n'est pas une archive Jarvis".to_string())?;
    let manifest: J = {
        let mut m = zip.by_name(MANIFEST).map_err(|_| "manifeste absent : ce n'est pas une archive Jarvis".to_string())?;
        let mut s = String::new();
        m.read_to_string(&mut s).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|_| "manifeste illisible".to_string())?
    };
    let format = manifest["format"].as_u64().unwrap_or(0);
    if format == 0 || format > crate::paths::DATA_FORMAT {
        return Err(format!("archive au format {format}, inconnu de cette version de Jarvis : mets Jarvis à jour"));
    }
    let sealer = if manifest["secrets"] == "chiffres" {
        let pw = password.filter(|p| !p.is_empty()).ok_or("cette archive est protégée par un mot de passe")?;
        let salt = b64().decode(manifest["sel"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
        let k = Sealer::new(pw, &salt)?;
        if k.open(manifest["verif"].as_str().unwrap_or("")).ok().as_deref() != Some(CHECK_TEXT) {
            return Err("mot de passe incorrect".into());
        }
        Some(k)
    } else {
        None
    };
    let expected: std::collections::HashMap<String, String> = manifest["fichiers"]
        .as_array()
        .map(|a| a.iter().map(|f| (f["chemin"].as_str().unwrap_or("").to_string(), f["sha256"].as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    let mut files = vec![];
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() || entry.name() == MANIFEST {
            continue;
        }
        let safe = entry.enclosed_name().map(|p| p.to_string_lossy().replace('\\', "/"));
        let rel = match safe.as_deref().and_then(|n| n.strip_prefix(PREFIX)) {
            Some(r) if !r.is_empty() && !r.contains(':') => r.to_string(),
            _ => return Err(format!("chemin refusé dans l'archive : {}", entry.name())),
        };
        let mut content = vec![];
        entry.read_to_end(&mut content).map_err(|e| e.to_string())?;
        if expected.get(&rel).map(String::as_str) != Some(sha(&content).as_str()) {
            return Err(format!("archive altérée ou incomplète : {rel}"));
        }
        files.push((rel, content));
    }
    if files.len() != expected.len() {
        return Err("archive incomplète : des fichiers du manifeste manquent".into());
    }
    Ok(Archive { manifest, files, sealer })
}

/// Prépare l'import dans `pending` (vidé d'abord) : fichiers écrits, secrets rechiffrés par `protect` (DPAPI).
pub fn stage(archive: &Archive, pending: &Path, protect: &dyn Fn(&str) -> Res<String>) -> Res<u64> {
    let _ = std::fs::remove_dir_all(pending);
    let write = || -> Res<u64> {
        for (rel, content) in &archive.files {
            let dst = pending.join(rel);
            if let Some(p) = dst.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut bytes = content.clone();
            if let (Some(k), true) = (&archive.sealer, rel.ends_with(".json") && String::from_utf8_lossy(content).contains(SEALED)) {
                if let Ok(mut v) = serde_json::from_slice::<J>(content) {
                    map_strings(&mut v, &mut |s| if s.starts_with(SEALED) { k.open(s).and_then(|c| protect(&c)).map(Some) } else { Ok(None) })?;
                    bytes = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
                }
            }
            std::fs::write(&dst, bytes).map_err(|e| format!("{rel} : {e}"))?;
        }
        Ok(archive.files.len() as u64)
    };
    write().inspect_err(|_| {
        let _ = std::fs::remove_dir_all(pending);
    })
}

/// Applique un import préparé : le dossier de données actuel devient `donnees.avant-import-<date>`, le dossier préparé
/// prend sa place, et l'état technique (modèles…) est repris de l'ancien. Jarvis doit être fermé (verrou d'instance).
pub fn apply_pending(data: &Path, pending: &Path) -> std::io::Result<Option<PathBuf>> {
    if !pending.is_dir() {
        return Ok(None);
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M").to_string();
    let name = data.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "donnees".into());
    let kept = data.with_file_name(format!("{name}.avant-import-{stamp}"));
    if data.is_dir() {
        std::fs::rename(data, &kept)?;
    }
    if let Err(err) = std::fs::rename(pending, data) {
        if kept.is_dir() {
            let _ = std::fs::rename(&kept, data);
        }
        return Err(err);
    }
    for d in crate::backup::SKIP_DIRS {
        let (from, to) = (kept.join(d), data.join(d));
        if *d != "runtime" && from.is_dir() && !to.exists() {
            let _ = std::fs::rename(&from, &to);
        }
    }
    if !data.join(crate::paths::VERSION_FILE).is_file() {
        let v = json!({ "format": crate::paths::DATA_FORMAT, "jarvis": env!("CARGO_PKG_VERSION"), "creeLe": chrono::Local::now().to_rfc3339() });
        std::fs::write(data.join(crate::paths::VERSION_FILE), serde_json::to_string_pretty(&v).unwrap_or_default())?;
    }
    Ok(kept.is_dir().then_some(kept))
}

/// Dossier où l'import attend le prochain lancement (à côté du dossier de données).
pub fn pending_dir(data: &Path) -> PathBuf {
    data.parent().map(|p| p.join(PENDING)).unwrap_or_else(|| data.join(PENDING))
}

/// Nom proposé pour une archive : Jarvis-donnees-AAAA-MM-JJ.jarvis.
pub fn default_name() -> String {
    format!("Jarvis-donnees-{}.{EXTENSION}", chrono::Local::now().format("%Y-%m-%d"))
}

// --- Ligne de commande -----------------------------------------------------------------------------------------------

/// `jarvis-core donnees …`
pub fn main(args: &[String]) -> i32 {
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let password = arg("--mot-de-passe");
    let target = args.get(1).filter(|a| !a.starts_with("--")).map(PathBuf::from);
    let data = crate::paths::data_dir();
    let sb = SecretBox::new();
    match (args.first().map(String::as_str), target) {
        (Some("exporter"), t) => {
            let out = t.unwrap_or_else(|| crate::backup::destination(&J::Null).join(default_name()));
            match export(&data, &out, password.as_deref(), &|s| sb.decrypt(s)) {
                Ok(r) => {
                    println!("{} fichier(s), {} Ko → {}", r.files, r.bytes / 1024, out.display());
                    if r.secrets > 0 {
                        println!("{} secret(s) {}", r.secrets, if r.sealed { "chiffrés par le mot de passe" } else { "retirés (pas de mot de passe) : à ressaisir après l'import" });
                    }
                    0
                }
                Err(e) => {
                    eprintln!("export impossible : {e}");
                    1
                }
            }
        }
        (Some(cmd @ ("apercu" | "importer")), Some(file)) => {
            let archive = match open(&file, password.as_deref()) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            };
            println!("{}", serde_json::to_string_pretty(&archive.preview()).unwrap_or_default());
            if cmd == "apercu" {
                return 0;
            }
            if !args.iter().any(|a| a == "--oui") {
                println!("Rien n'a été modifié. Ajoute --oui pour remplacer tes données par celles de l'archive (l'actuel est gardé à côté).");
                return 0;
            }
            let pending = pending_dir(&crate::paths::target_data_dir());
            if let Err(e) = stage(&archive, &pending, &|s| sb.encrypt(s)) {
                eprintln!("import impossible : {e}");
                return 1;
            }
            match crate::paths::relocate_guarded() {
                Ok(_) if !pending.exists() => println!("Import appliqué : {}", data.display()),
                _ => println!("Import préparé : il sera appliqué au prochain lancement de Jarvis."),
            }
            0
        }
        _ => {
            eprintln!("usage : jarvis-core donnees exporter [fichier.jarvis] [--mot-de-passe m] | apercu <fichier> | importer <fichier> --oui");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jarvis-test-transfert-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // DPAPI simulé : « AQAAANCMnd8BFdERjHoAwE/Cl+s » + texte clair inversé.
    fn fake_protect(s: &str) -> Res<String> {
        Ok(format!("{DPAPI_B64}{}", s.chars().rev().collect::<String>()))
    }
    fn fake_unprotect(s: &str) -> Res<String> {
        Ok(s.trim_start_matches(DPAPI_B64).chars().rev().collect())
    }

    fn sample(data: &Path) {
        std::fs::create_dir_all(data.join("collections").join("images")).unwrap();
        std::fs::create_dir_all(data.join("models")).unwrap();
        std::fs::create_dir_all(data.join("runtime")).unwrap();
        std::fs::write(data.join("memory.json"), "{\"souvenirs\":[\"chat\"]}").unwrap();
        std::fs::write(data.join("collections").join("images").join("a.jpg"), [0u8, 1, 2]).unwrap();
        std::fs::write(data.join("models").join("gros.onnx"), "x").unwrap();
        std::fs::write(data.join("runtime").join("noyau.lock"), "x").unwrap();
        std::fs::write(data.join("google.json"), format!("{{\"appPasswordEnc\":\"{}\"}}", fake_protect("motdepasse-google").unwrap())).unwrap();
    }

    #[test]
    fn export_puis_import_avec_mot_de_passe() {
        let t = tmp("aller-retour");
        let (data, arch) = (t.join("donnees"), t.join("a.jarvis"));
        sample(&data);
        let r = export(&data, &arch, Some("secret"), &fake_unprotect).unwrap();
        assert_eq!((r.files, r.secrets), (3, 1));
        // Le secret n'est jamais en clair dans l'archive.
        let raw = std::fs::read(&arch).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("motdepasse-google"));
        assert!(open(&arch, Some("faux")).err().unwrap().contains("mot de passe"));
        assert!(open(&arch, None).is_err());
        let a = open(&arch, Some("secret")).unwrap();
        assert_eq!(a.preview()["fichiers"], 3);
        let pending = pending_dir(&data);
        stage(&a, &pending, &fake_protect).unwrap();
        let kept = apply_pending(&data, &pending).unwrap().unwrap();
        let g: J = serde_json::from_str(&std::fs::read_to_string(data.join("google.json")).unwrap()).unwrap();
        assert_eq!(fake_unprotect(g["appPasswordEnc"].as_str().unwrap()).unwrap(), "motdepasse-google");
        assert_eq!(std::fs::read(data.join("collections").join("images").join("a.jpg")).unwrap(), vec![0u8, 1, 2]);
        // Modèles repris de l'ancien dossier, gardé à côté ; plus rien en attente.
        assert!(data.join("models").join("gros.onnx").is_file() && kept.join("memory.json").is_file() && !pending.exists());
        assert!(data.join(crate::paths::VERSION_FILE).is_file());
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn sans_mot_de_passe_les_secrets_sont_retires() {
        let t = tmp("sans-mdp");
        let (data, arch) = (t.join("donnees"), t.join("a.jarvis"));
        sample(&data);
        export(&data, &arch, None, &fake_unprotect).unwrap();
        let a = open(&arch, None).unwrap();
        assert_eq!(a.manifest["secretsRetiresDe"], json!(["google.json"]));
        let g = a.files.iter().find(|(r, _)| r == "google.json").unwrap();
        assert!(!String::from_utf8_lossy(&g.1).contains(DPAPI_B64));
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn archive_alteree_refusee() {
        let t = tmp("alteree");
        let (data, arch, bad) = (t.join("donnees"), t.join("a.jarvis"), t.join("b.jarvis"));
        sample(&data);
        export(&data, &arch, None, &fake_unprotect).unwrap();
        // Réécrit l'archive en modifiant memory.json sans toucher au manifeste.
        let mut src = zip::ZipArchive::new(std::fs::File::open(&arch).unwrap()).unwrap();
        let mut w = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
        for i in 0..src.len() {
            let mut e = src.by_index(i).unwrap();
            let mut c = vec![];
            e.read_to_end(&mut c).unwrap();
            if e.name() == "donnees/memory.json" {
                c = b"{\"pirate\":1}".to_vec();
            }
            w.start_file(e.name(), zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&c).unwrap();
        }
        w.finish().unwrap();
        assert!(open(&bad, None).err().unwrap().contains("altérée"));
        let _ = std::fs::remove_dir_all(&t);
    }
}
