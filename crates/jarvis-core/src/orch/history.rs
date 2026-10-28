// Journal de Jarvis (orchestrator/history.js) : tout ce qu'il fait y est inscrit, en ajout seul.
//
// Trois protections se cumulent :
//   1. le fichier n'est jamais réécrit, seulement complété ;
//   2. chaque ligne contient l'empreinte de la précédente : toute suppression ou modification a
//      posteriori casse la chaîne et devient détectable ;
//   3. les droits Windows du fichier n'autorisent que l'ajout et la lecture.
//
// Format identique à la v3 (même empreinte SHA-256 sur `seq|at|type|JSON(data)|prev`) : le journal
// de la v3 se prolonge tel quel. Embranchements (deux noyaux sur les mêmes données) : acceptés une
// fois constatés dans le journal lui-même (entrée « embranchement_constate »).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::js::{self, J};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const FORK_TYPE: &str = "embranchement_constate";

pub fn fingerprint(record: &J) -> String {
    let seq = js::s(&record["seq"]);
    let data = serde_json::to_string(&record["data"]).unwrap_or_else(|_| "null".into());
    let text = format!("{}|{}|{}|{}|{}", seq, js::gs(record, "at"), js::gs(record, "type"), data, js::gs(record, "prev"));
    hex::encode(Sha256::digest(text.as_bytes()))
}

pub struct History {
    pub file: PathBuf,
    log: Rc<dyn Fn(String)>,
    seal: bool,
    pub seq: Cell<i64>,
    pub last: RefCell<String>,
    cache: RefCell<Option<Vec<J>>>,
    cache_size: Cell<i64>,
    verified_size: Cell<Option<i64>>,
    known_size: Cell<Option<i64>>,
}

impl History {
    // seal : activé seulement par l'orchestrateur ; les outils de lecture ne touchent pas aux droits.
    pub fn new(file: PathBuf, log: Rc<dyn Fn(String)>, seal: bool) -> Self {
        let h = Self {
            file,
            log,
            seal,
            seq: Cell::new(0),
            last: RefCell::new(GENESIS.into()),
            cache: RefCell::new(None),
            cache_size: Cell::new(-1),
            verified_size: Cell::new(None),
            known_size: Cell::new(None),
        };
        h.load();
        h
    }

    fn load(&self) {
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if !self.file.exists() {
            let _ = std::fs::write(&self.file, "");
        }
        let text = std::fs::read_to_string(&self.file).unwrap_or_default();
        let lines: Vec<&str> = text.split('\n').filter(|l| !l.is_empty()).collect();
        self.take_tail(&lines);
        self.known_size.set(Some(self.size()));
        if self.seal {
            self.harden();
        }
    }

    // Dernière entrée lisible de ces lignes : la chaîne continue à partir d'elle.
    fn take_tail(&self, lines: &[&str]) -> bool {
        for line in lines.iter().rev() {
            if let Ok(record) = serde_json::from_str::<J>(line) {
                self.seq.set(record["seq"].as_i64().unwrap_or(0));
                *self.last.borrow_mut() = js::gs(&record, "hash");
                return true;
            }
        }
        false
    }

    // Le fichier a grandi sans nous : on repart de sa vraie fin pour que la chaîne reste unique.
    fn resync(&self) {
        let res = (|| -> std::io::Result<bool> {
            let size = self.size().max(0) as u64;
            let start = size.saturating_sub(65536);
            let mut f = std::fs::File::open(&self.file)?;
            f.seek(SeekFrom::Start(start))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            let text = String::from_utf8_lossy(&buf).into_owned();
            let mut lines: Vec<&str> = text.split('\n').filter(|l| !l.is_empty()).collect();
            if start > 0 && !lines.is_empty() {
                lines.remove(0);
            }
            Ok(self.take_tail(&lines))
        })();
        match res {
            Ok(true) => (self.log)(format!("journal : écriture d'un autre processus détectée, chaîne recalée sur l'entrée {}", self.seq.get())),
            Ok(false) => {}
            Err(err) => (self.log)(format!("journal : recalage impossible ({err})")),
        }
    }

    // Droits Windows : ajout et lecture seulement (fichier : pas d'écrasement ni de suppression ;
    // dossier : pas de suppression d'enfant, donc ni suppression ni renommage du journal).
    fn harden(&self) {
        let Some(user) = js::env("USERNAME") else { return };
        let dir = self.file.parent().map(|d| d.to_path_buf()).unwrap_or_default();
        let run = |args: Vec<String>| -> std::io::Result<bool> {
            use std::os::windows::process::CommandExt;
            let st = std::process::Command::new("icacls")
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x0800_0000)
                .status()?;
            Ok(st.success())
        };
        let a = run(vec![
            self.file.to_string_lossy().into_owned(),
            "/inheritance:r".into(),
            "/grant".into(),
            format!("{user}:(RD,REA,RA,S,AD,WA,WEA)"),
            "/grant".into(),
            "SYSTEM:(F)".into(),
        ]);
        let b = run(vec![dir.to_string_lossy().into_owned(), "/inheritance:r".into(), "/grant".into(), format!("{user}:(RX,WD,AD)"), "/grant".into(), "SYSTEM:(F)".into()]);
        match (a, b) {
            (Ok(true), Ok(true)) => (self.log)("journal scellé : ajout et lecture uniquement".into()),
            (Err(e), _) | (_, Err(e)) => (self.log)(format!("droits du journal non appliqués : {e}")),
            _ => (self.log)("droits du journal non appliqués : icacls a échoué".into()),
        }
    }

    pub fn append(&self, typ: &str, data: J) -> J {
        if let Some(k) = self.known_size.get() {
            if self.size() != k {
                self.resync();
            }
        }
        let mut record = json!({
            "seq": self.seq.get() + 1,
            "at": js::iso_now(),
            "type": typ,
            "data": if data.is_null() { json!({}) } else { data },
            "prev": self.last.borrow().clone(),
        });
        let hash = fingerprint(&record);
        record["hash"] = json!(hash);
        let line = format!("{}\n", serde_json::to_string(&record).unwrap_or_default());
        let before = if self.cache.borrow().is_some() || self.verified_size.get().is_some() { self.size() } else { -2 };
        let written = std::fs::OpenOptions::new().create(true).append(true).open(&self.file).and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(err) = written {
            (self.log)(format!("journal : écriture impossible ({err})"));
        }
        self.seq.set(record["seq"].as_i64().unwrap_or(0));
        *self.last.borrow_mut() = hash;
        self.known_size.set(Some(self.size()));
        let len = line.len() as i64;
        match self.verified_size.get() {
            Some(v) if before == v => self.verified_size.set(Some(before + len)),
            _ => self.verified_size.set(None),
        }
        let mut cache = self.cache.borrow_mut();
        if cache.is_some() && before == self.cache_size.get() {
            cache.as_mut().unwrap().push(record.clone());
            self.cache_size.set(before + len);
        } else {
            *cache = None;
        }
        record
    }

    pub fn size(&self) -> i64 {
        std::fs::metadata(&self.file).map(|m| m.len() as i64).unwrap_or(-1)
    }

    pub fn read(&self, fresh: bool) -> Vec<J> {
        if !self.file.exists() {
            return vec![];
        }
        let size = self.size();
        if !fresh {
            if let Some(c) = self.cache.borrow().as_ref() {
                if size == self.cache_size.get() {
                    return c.clone();
                }
            }
        }
        let text = std::fs::read_to_string(&self.file).unwrap_or_default();
        let records: Vec<J> = text.split('\n').filter(|l| !l.is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect();
        *self.cache.borrow_mut() = Some(records.clone());
        self.cache_size.set(size);
        records
    }

    /// Recherche paginée (administration), plus récentes d'abord.
    pub fn query(&self, types: Option<&[String]>, text: &str, before: Option<i64>, limit: usize) -> Vec<J> {
        let records = self.read(false);
        let wanted: Option<HashSet<&String>> = types.filter(|t| !t.is_empty()).map(|t| t.iter().collect());
        let needle = text.trim().to_lowercase();
        let mut out = vec![];
        for r in records.iter().rev() {
            if out.len() >= limit {
                break;
            }
            let seq = r["seq"].as_i64().unwrap_or(0);
            if let Some(b) = before {
                if seq >= b {
                    continue;
                }
            }
            let typ = js::gs(r, "type");
            if let Some(w) = &wanted {
                if !w.contains(&typ) {
                    continue;
                }
            }
            if !needle.is_empty() && !(typ.contains(&needle) || serde_json::to_string(&r["data"]).unwrap_or_default().to_lowercase().contains(&needle)) {
                continue;
            }
            out.push(r.clone());
        }
        out
    }

    pub fn counts(&self) -> J {
        let mut counts = serde_json::Map::new();
        for r in self.read(false) {
            let t = js::gs(&r, "type");
            let n = counts.get(&t).and_then(|v| v.as_i64()).unwrap_or(0);
            counts.insert(t, json!(n + 1));
        }
        Value::Object(counts)
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<J> {
        let records = self.read(false);
        let needle = query.trim().to_lowercase();
        let matching: Vec<J> = if needle.is_empty() {
            records
        } else {
            records
                .into_iter()
                .filter(|r| serde_json::to_string(&r["data"]).unwrap_or_default().to_lowercase().contains(&needle) || js::gs(r, "type").contains(&needle))
                .collect()
        };
        let skip = matching.len().saturating_sub(limit);
        matching.into_iter().skip(skip).collect()
    }

    // Parcourt toute la chaîne : chaque entrée doit avoir une empreinte valide et descendre d'une
    // entrée présente. Une entrée qui ne prolonge pas la ligne précédente forme un embranchement.
    fn analyze(records: &[J]) -> Result<(HashMap<String, i64>, Vec<usize>, i64), (i64, &'static str)> {
        let mut by_hash: HashMap<String, i64> = HashMap::new();
        let mut forks = vec![];
        let mut max_seq = 0;
        for (i, record) in records.iter().enumerate() {
            let seq = record["seq"].as_i64().unwrap_or(0);
            let hash = js::gs(record, "hash");
            if fingerprint(record) != hash {
                return Err((seq, "contenu modifié"));
            }
            let prev = js::gs(record, "prev");
            let parent = if prev == GENESIS { Some(0) } else { by_hash.get(&prev).copied() };
            let Some(parent_seq) = parent else { return Err((seq, "chaînage rompu")) };
            if seq != parent_seq + 1 {
                return Err((seq, "entrée manquante"));
            }
            let expected = if i == 0 { GENESIS.to_string() } else { js::gs(&records[i - 1], "hash") };
            if prev != expected {
                forks.push(i);
            }
            by_hash.insert(hash, seq);
            max_seq = max_seq.max(seq);
        }
        Ok((by_hash, forks, max_seq))
    }

    /// Recalcule toute la chaîne : dit si le journal a été altéré, et où.
    pub fn verify(&self) -> J {
        self.verify_expected(None)
    }

    pub fn verify_expected(&self, expected_count: Option<usize>) -> J {
        let records = self.read(true);
        let count = records.len();
        let fail = |at: Option<i64>, reason: String| {
            let mut v = json!({ "ok": false, "count": count, "reason": reason });
            if let Some(a) = at {
                v["at"] = json!(a);
            }
            v
        };
        if let Some(c) = expected_count {
            if count != c {
                return fail(None, format!("troncature : {count} entrées trouvées au lieu de {c}"));
            }
        }
        let (by_hash, forks, max_seq) = match Self::analyze(&records) {
            Ok(x) => x,
            Err((at, reason)) => return fail(Some(at), reason.into()),
        };
        if self.seq.get() > 0 && max_seq < self.seq.get() {
            return fail(None, format!("troncature : entrée {max_seq} trouvée au plus loin, {} en mémoire", self.seq.get()));
        }
        let mut acknowledged = HashSet::new();
        for record in &records {
            if js::gs(record, "type") != FORK_TYPE {
                continue;
            }
            if let Some(list) = record["data"]["entrees"].as_array() {
                for h in list {
                    let h = js::s(h);
                    if !by_hash.contains_key(&h) {
                        return fail(record["seq"].as_i64(), "entrée manquante (embranchement constaté)".into());
                    }
                    acknowledged.insert(h);
                }
            }
        }
        let unexplained: Vec<usize> = forks.iter().copied().filter(|i| !acknowledged.contains(&js::gs(&records[*i], "hash"))).collect();
        if !unexplained.is_empty() {
            let mut v = fail(records[unexplained[0]]["seq"].as_i64(), "chaînage rompu (embranchement non constaté)".into());
            v["forks"] = json!(unexplained.len());
            return v;
        }
        self.verified_size.set(Some(self.cache_size.get()));
        let mut ok = json!({ "ok": true, "count": count });
        if !forks.is_empty() {
            ok["forks"] = json!(forks.len());
        }
        ok
    }

    /// Embranchement non encore constaté, sans altération et terminé depuis plus d'une minute :
    /// inscrit une fois dans le journal. Retourne l'entrée ajoutée.
    pub fn acknowledge_forks(&self) -> Option<J> {
        let now = js::now_ms();
        let records = self.read(true);
        let (_, forks, _) = Self::analyze(&records).ok()?;
        if forks.is_empty() {
            return None;
        }
        let mut acknowledged = HashSet::new();
        for record in &records {
            if js::gs(record, "type") == FORK_TYPE {
                if let Some(list) = record["data"]["entrees"].as_array() {
                    for h in list {
                        acknowledged.insert(js::s(h));
                    }
                }
            }
        }
        let pending: Vec<usize> = forks.into_iter().filter(|i| !acknowledged.contains(&js::gs(&records[*i], "hash"))).collect();
        if pending.is_empty() {
            return None;
        }
        let last = &records[*pending.last().unwrap()];
        if now - js::parse_date_ms(&js::gs(last, "at")).unwrap_or(now) < 60000 {
            return None;
        }
        let index: HashMap<String, usize> = records.iter().enumerate().map(|(i, r)| (js::gs(r, "hash"), i)).collect();
        let mut from = pending[0];
        for &i in &pending {
            from = from.min(*index.get(&js::gs(&records[i], "prev")).unwrap_or(&i));
        }
        let to = *pending.last().unwrap();
        let zone = &records[from..=to];
        Some(self.append(
            FORK_TYPE,
            json!({
                "debut": zone[0]["at"], "fin": zone[zone.len() - 1]["at"], "premiere": zone[0]["seq"],
                "entrees": zone.iter().map(|r| r["hash"].clone()).collect::<Vec<_>>(),
                "bifurcations": pending.len(),
                "motif": "écritures simultanées de plusieurs noyaux sur les mêmes données (aucune entrée modifiée ni supprimée)"
            }),
        ))
    }

    /// Vérification rapide : si le fichier n'a bougé que par nos propres ajouts depuis la dernière
    /// vérification complète, la chaîne reste intègre ; sinon vérification complète.
    pub fn verify_cached(&self) -> J {
        if let Some(v) = self.verified_size.get() {
            if v == self.size() {
                return json!({ "ok": true, "count": self.seq.get(), "cached": true });
            }
        }
        self.verify()
    }
}
