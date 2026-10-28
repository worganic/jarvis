// « Savoir » : dossiers de recherche sur un thème ou un produit (« fais un dossier sur l'hélicoptère H160 »).
// Claude Code part en tâche de fond avec un temps limité (WebSearch / WebFetch) et écrit dans le dossier de travail
// (workspace/savoir/<slug>/) un dossier JSON structuré et un rapport ; le noyau relit le JSON (dossier.rs), télécharge
// les images et le modèle 3D (adresses publiques seulement), fusionne avec ce qui existe et indexe le tout dans un
// RAG propre au sujet (data/savoir/<slug>/vectors.json, même moteur que les listes). La card « Data » (module savoir)
// liste les sujets ; l'IA les interroge par l'outil savoir_chercher (lecture seule). L'IA n'écrit jamais dans data/.

pub mod dossier;
pub mod explore;
pub mod sketchfab;
pub mod present;
pub mod profils;
pub mod voice;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use base64::Engine;
use serde_json::json;

use super::collections::enrich::{download_image, host_is_public};
use super::collections::index::{hash, Index, Passage};
use super::embeddings::Embedder;
use super::{OrchRef, Up};
use crate::js::{self, J};

pub const DEFAULT_MINUTES: u32 = 5;
pub const MAX_MINUTES: u32 = 30;
const MAX_DOSSIERS: usize = 100;
const DOSSIER_MAX_BYTES: u64 = 300 * 1024 * 1024;
const GLB_MAX_BYTES: usize = 50 * 1024 * 1024;
/// Délai accordé après le budget avant d'arrêter la recherche.
const GRACE_MS: i64 = 2 * 60 * 1000;
const WATCH_MS: u64 = 3_000;
/// Outils intégrés laissés à Claude Code pendant une recherche.
pub const SEARCH_TOOLS: &str = "WebSearch,WebFetch,Write,Read,Edit";

#[derive(Clone)]
pub struct Job {
    pub slug: String,
    pub sujet: String,
    /// Axe d'approfondissement ou lien à lire (vide : création).
    pub axe: String,
    pub minutes: u32,
    pub json_file: PathBuf,
    pub md_file: PathBuf,
}

pub struct Savoir {
    o: OrchRef,
    dir: PathBuf,
    log: Rc<dyn Fn(String)>,
    embedder: RefCell<Option<Rc<Embedder>>>,
    indexes: RefCell<HashMap<String, Rc<Index>>>,
    dossiers: RefCell<BTreeMap<String, J>>,
    /// consigne envoyée à l'IA → recherche en cours
    pending: RefCell<HashMap<String, Job>>,
    /// Présentation en cours (une seule à la fois).
    presentation: RefCell<Option<present::Presentation>>,
    /// Explorateur 3D ou écran médias ouvert dans le HUD (dossier montré).
    exploring: RefCell<Option<String>>,
    /// Dossiers dont la recherche démarre (profil en cours de choix).
    starting: RefCell<std::collections::HashSet<String>>,
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| match e.metadata() { Ok(m) if m.is_dir() => dir_size(&e.path()), Ok(m) => m.len(), Err(_) => 0 }).sum())
        .unwrap_or(0)
}

fn words(text: &str) -> Vec<String> {
    const STOP: &[&str] = &["les", "des", "une", "sur", "pour", "avec", "dans", "the", "aux", "du", "de", "la", "le", "un", "et"];
    dossier::ident(text, 200).split('-').filter(|w| (w.len() >= 3 || w.chars().any(|c| c.is_ascii_digit())) && !STOP.contains(w)).map(String::from).collect()
}

/// Modèle 3D glTF binaire : https public, 50 Mo au plus, en-tête « glTF ».
async fn download_glb(url: &str) -> Result<Vec<u8>, String> {
    if !host_is_public(url) {
        return Err("adresse refusée".into());
    }
    let mut resp = crate::net::http().get(url).timeout(std::time::Duration::from_secs(120)).send().await.map_err(|e| format!("réseau : {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("introuvable ({})", resp.status().as_u16()));
    }
    if !host_is_public(resp.url().as_str()) {
        return Err("redirection refusée".into());
    }
    if resp.content_length().map(|l| l as usize > GLB_MAX_BYTES).unwrap_or(false) {
        return Err("modèle trop lourd".into());
    }
    let mut bytes: Vec<u8> = vec![];
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("réseau : {e}"))? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > GLB_MAX_BYTES {
            return Err("modèle trop lourd".into());
        }
    }
    if bytes.len() < 20 || &bytes[..4] != b"glTF" {
        return Err("ce n'est pas un modèle glTF binaire".into());
    }
    Ok(bytes)
}

impl Savoir {
    pub fn new(o: OrchRef, dir: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let me = Self { o, dir, log, embedder: RefCell::new(None), indexes: RefCell::new(HashMap::new()), dossiers: RefCell::new(BTreeMap::new()), pending: RefCell::new(HashMap::new()), presentation: RefCell::new(None), exploring: RefCell::new(None), starting: RefCell::new(Default::default()) };
        me.load();
        me
    }

    fn load(&self) {
        let mut all = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten().filter(|e| e.path().is_dir()) {
                let slug = e.file_name().to_string_lossy().to_string();
                if let Some(d) = js::read_json(&e.path().join("dossier.json")) {
                    all.insert(slug, d);
                }
            }
        }
        // Recherches coupées par un arrêt de Jarvis : signalées « interrompues », relançables.
        for job in js::read_json(&self.jobs_file()).and_then(|v| v.as_array().cloned()).unwrap_or_default() {
            if let Some(d) = all.get_mut(&js::gs(&job, "slug")) {
                d["meta"]["etat"] = json!("interrompu");
                let _ = js::write_json_atomic(&self.dir.join(js::gs(&job, "slug")).join("dossier.json"), d);
            }
        }
        let _ = std::fs::remove_file(self.jobs_file());
        *self.dossiers.borrow_mut() = all;
    }

    fn jobs_file(&self) -> PathBuf {
        self.dir.join("jobs.json")
    }

    fn save_jobs(&self) {
        let jobs: Vec<J> = self.pending.borrow().values().map(|j| json!({ "slug": j.slug, "sujet": j.sujet, "axe": j.axe })).collect();
        if jobs.is_empty() {
            let _ = std::fs::remove_file(self.jobs_file());
        } else {
            let _ = js::write_json_atomic(&self.jobs_file(), &json!(jobs));
        }
    }

    pub fn attach_embedder(&self, e: Option<Rc<Embedder>>) {
        *self.embedder.borrow_mut() = e.clone();
        for idx in self.indexes.borrow().values() {
            idx.attach_embedder(e.clone());
        }
    }

    fn index(&self, slug: &str) -> Rc<Index> {
        if let Some(i) = self.indexes.borrow().get(slug) {
            return i.clone();
        }
        let idx = Rc::new(Index::new(self.dir.join(slug).join("vectors.json")));
        idx.attach_embedder(self.embedder.borrow().clone());
        self.indexes.borrow_mut().insert(slug.to_string(), idx.clone());
        idx
    }

    fn save(&self, slug: &str) {
        if let Some(d) = self.dossiers.borrow().get(slug) {
            let _ = std::fs::create_dir_all(self.dir.join(slug));
            let _ = js::write_json_atomic(&self.dir.join(slug).join("dossier.json"), d);
        }
    }

    /// Card de suivi : étape en cours, chiffres, ce qui vient d'être recueilli ou enregistré.
    fn progress(&self, slug: &str, data: J) {
        let mut ev = json!({ "type": "savoir_progress", "slug": slug, "at": js::now_ms() });
        js::assign(&mut ev, data);
        self.o.up().emit(ev);
    }

    fn changed(&self) {
        self.o.up().emit(json!({ "type": "savoir_changed" }));
    }

    pub fn get(&self, slug: &str) -> Option<J> {
        self.dossiers.borrow().get(slug).cloned()
    }

    /// Outils de Claude Code pour une recherche en cours : web et écriture dans le dossier de travail, jamais de
    /// commande système (elle demanderait un code à chaque appel et bloquerait la recherche).
    pub fn tools_for(&self, text: &str) -> Option<&'static str> {
        self.pending.borrow().contains_key(text).then_some(SEARCH_TOOLS)
    }

    pub fn is_running(&self, slug: &str) -> bool {
        self.starting.borrow().contains(slug) || self.pending.borrow().values().any(|j| j.slug == slug)
    }

    /// Dossier désigné par un nom (« le H160 », « hélicoptère h160 ») : le plus de mots en commun.
    pub fn find(&self, name: &str) -> Option<String> {
        let slug = dossier::ident(name, 60);
        if self.dossiers.borrow().contains_key(&slug) {
            return Some(slug);
        }
        let wanted = words(name);
        if wanted.is_empty() {
            return None;
        }
        self.dossiers
            .borrow()
            .iter()
            .map(|(s, d)| {
                let have = words(&format!("{s} {}", js::gs(d, "sujet")));
                (wanted.iter().filter(|w| have.contains(w)).count(), s.clone())
            })
            .filter(|(n, _)| *n > 0)
            .max_by_key(|(n, _)| *n)
            .map(|(_, s)| s)
    }

    fn rapports(&self, slug: &str) -> Vec<String> {
        let dir = self.dir.join(slug).join("rapports");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "md").unwrap_or(false)).collect()).unwrap_or_default();
        files.sort();
        files.iter().filter_map(|p| std::fs::read_to_string(p).ok()).map(|t| js::slice_chars(&t, 200_000)).collect()
    }

    fn passages(&self, slug: &str) -> Vec<Passage> {
        let Some(d) = self.get(slug) else { return vec![] };
        dossier::passages_text(&d, &self.rapports(slug))
            .into_iter()
            .enumerate()
            .map(|(i, text)| Passage { list: slug.to_string(), element: format!("{slug}#{i}"), text, ids: vec![] })
            .collect()
    }

    // --- Lancer une recherche --------------------------------------------------------------------------

    fn prompt(job: &Job, existing: Option<&J>, profil: &J) -> String {
        let m = job.minutes;
        let (sources, images) = ((m * 3).clamp(5, 30), (m * 6).clamp(8, 60));
        let mut p = vec![];
        if job.axe.is_empty() {
            p.push(format!("Constitue un dossier documentaire complet sur « {} ».", job.sujet));
        } else if crate::re!(r"^https?://").is_match(&job.axe) {
            p.push(format!("Complète le dossier documentaire sur « {} » à partir de cette page : {} (lis-la avec WebFetch ; si c'est une vidéo, renseigne-la dans videos avec un résumé de son contenu).", job.sujet, job.axe));
        } else {
            p.push(format!("Complète le dossier documentaire sur « {} » en cherchant surtout : {}.", job.sujet, job.axe));
        }
        if let Some(d) = existing {
            let known: Vec<String> = d["sources"].as_array().map(|a| a.iter().take(30).map(|s| js::gs(s, "url")).collect()).unwrap_or_default();
            if !known.is_empty() {
                p.push(format!("Le dossier existe déjà : n'y remets que du nouveau. Sources déjà lues : {}.", known.join(", ")));
            }
        }
        p.push(format!("Tu as environ {m} minutes : vise {sources} sources fiables et {images} images, puis arrête-toi."));
        p.push("Utilise la recherche web (WebSearch puis WebFetch) : sites officiels, encyclopédies, presse spécialisée, fiches techniques ; croise les chiffres entre les sources.".into());
        p.push("N'utilise que WebSearch, WebFetch et l'écriture de fichiers : jamais de commande système (Bash, curl, PowerShell), ni d'autre outil. Pour les images, lis avec WebFetch les pages de Wikimedia Commons ou des sites officiels et relève l'adresse directe des fichiers (upload.wikimedia.org…). Si un outil est refusé, continue sans lui.".into());
        let consigne = js::gs(profil, "consigne");
        if !consigne.is_empty() {
            p.push(consigne);
        }
        p.push(format!("Écris en français, dans le fichier {} (crée le dossier au besoin), un seul objet JSON avec ces clés :", job.json_file.to_string_lossy()));
        let mut keys: Vec<&str> = vec![
            r#""sujet" ; "resume" (une dizaine de lignes) ; "categorie" ; "fiche" [{"cle","valeur","unite","source"}] (caractéristiques chiffrées) ; "sections" [{"titre","texte","sources"}] (histoire, conception, fonctionnement, utilisations, variantes, limites…) ; "chronologie" [{"date","fait"}] ; "comparaisons" [{"autre","points"}] (équivalents ou concurrents)"#,
            r#""images" [{"url","legende","vue","licence","source"}] (adresses https directes vers des fichiers .jpg, .png ou .webp, de préférence Wikimedia Commons ou sites officiels ; vue parmi profil, face, dessus, detail, cockpit, interieur, action, schema, autre ; varie les vues)"#,
            r#""videos" [{"url","titre","duree","resume"}] (adresses directes des vidéos, YouTube de préférence)"#,
        ];
        let champs = js::str_list(&profil["champs"]);
        for (champ, text) in [
            ("points_forts", r#""points_forts" ["…"] (5 à 10 phrases courtes, d'après les tests, la critique ou les utilisateurs)"#),
            ("points_faibles", r#""points_faibles" ["…"] (idem)"#),
            ("prix", r#""prix" [{"vendeur","prix","devise","url","date"}] (prix relevés sur des sites marchands ou officiels ; prix en nombre ; date du relevé)"#),
            ("avis", r#""avis" [{"source","note","sur","resume","url"}] (tests de la presse ou notes du public ; note et barème, par exemple 4,5 sur 5)"#),
            ("ecoute", r#""ecoute" [{"titre","type","artiste","annee","spotify","youtube"}] (type parmi album, titre, concert, playlist, bande originale ; liens directs open.spotify.com et YouTube)"#),
            ("pieces", r#""pieces" [{"id","nom","description","position"}] (éléments visibles qu'on pourrait montrer sur un modèle 3D, position parmi avant, arriere, haut, bas, gauche, droite, centre)"#),
            ("modeles3d", r#""modeles3d" [{"url_page","url_glb","licence","auteur"}] (modèles 3D sous licence libre, Sketchfab, NASA… ; url_glb seulement pour un lien direct vers un fichier .glb)"#),
            ("maquette", r#""maquette" ({"echelle","elements":[{"forme","dimensions","position","rotation","couleur","piece","anime"}]} : maquette simplifiée en 20 à 80 formes ; forme parmi cylindre, sphere, boite, capsule, cone, tore ; dimensions, position et rotation en tableaux de 3 nombres, en mètres et radians ; couleur #rrggbb ; piece = id d'une pièce ; anime parmi rotation_x, rotation_y, rotation_z pour ce qui tourne)"#),
        ] {
            if champs.iter().any(|c| c == champ) {
                keys.push(text);
            }
        }
        keys.push(r#""sources" [{"titre","url"}]"#);
        p.push(format!("{}.", keys.join(" ; ")));
        p.push("Écris une première version du fichier dès tes premières sources, puis complète-le au fil de la recherche : si le temps manque, ce qui est écrit sera gardé.".into());
        p.push(format!("Écris aussi un rapport Markdown détaillé et sourcé dans {}. N'écris aucun autre fichier.", job.md_file.to_string_lossy()));
        p.push("Ta réponse finale est lue à voix haute : deux phrases au plus, sans markdown.".into());
        p.join(" ")
    }

    /// Choisit le profil du dossier (mots-clés, sinon Haiku qui peut en créer un), puis lance la recherche avec la
    /// consigne de ce profil.
    async fn launch(&self, job: Job, existing: Option<J>) {
        self.progress(&job.slug, json!({ "phase": "attente", "note": "choix du profil du dossier (produit, musique, film, lieu…)" }));
        let profil = self.ensure_profile(&job.slug).await;
        self.starting.borrow_mut().remove(&job.slug);
        self.progress(&job.slug, json!({ "phase": "attente", "note": format!("profil : {} — la recherche démarre", js::gs(&profil, "nom")), "profil": profil["nom"] }));
        let text = Self::prompt(&job, existing.as_ref(), &profil);
        self.pending.borrow_mut().insert(text.clone(), job);
        self.save_jobs();
        self.o.up().enqueue_ai(text.clone(), json!({ "background": true }));
        self.watch(text);
    }

    /// Lance la recherche (création, approfondissement ou lien à lire).
    pub fn start(&self, sujet: &str, slug_hint: Option<&str>, axe: &str, minutes: u32) -> J {
        let o = self.o.up();
        let slug = slug_hint.map(String::from).unwrap_or_else(|| dossier::ident(sujet, 60));
        if slug.is_empty() {
            return json!({ "ok": false, "say": "Je n'ai pas compris le sujet du dossier." });
        }
        if self.is_running(&slug) {
            return json!({ "ok": false, "say": format!("Une recherche est déjà en cours sur {sujet}.") });
        }
        let existing = self.get(&slug);
        if existing.is_none() && self.dossiers.borrow().len() >= MAX_DOSSIERS {
            return json!({ "ok": false, "say": format!("Tu as déjà {MAX_DOSSIERS} dossiers : supprimes-en un d'abord.") });
        }
        let sujet = existing.as_ref().map(|d| js::gs(d, "sujet")).filter(|s| !s.is_empty()).unwrap_or_else(|| sujet.to_string());
        let minutes = minutes.clamp(1, MAX_MINUTES);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let work = crate::paths::workspace().join("savoir").join(&slug);
        let job = Job {
            slug: slug.clone(),
            sujet: sujet.clone(),
            axe: axe.trim().to_string(),
            minutes,
            json_file: work.join(format!("dossier-{stamp}.json")),
            md_file: work.join(format!("rapport-{stamp}.md")),
        };
        {
            let mut all = self.dossiers.borrow_mut();
            let d = all.entry(slug.clone()).or_insert_with(|| json!({ "sujet": sujet, "meta": { "cree": js::iso_now(), "questions": 0, "presentations": 0, "recherches": [] } }));
            d["meta"]["etat"] = json!("en_cours");
            d["meta"]["debut"] = json!(js::iso_now());
            d["meta"]["budget"] = json!(minutes);
        }
        self.save(&slug);
        self.starting.borrow_mut().insert(slug.clone());
        o.record("savoir_recherche", json!({ "dossier": slug, "minutes": minutes, "approfondissement": !job.axe.is_empty() }));
        let known = existing.as_ref().map(dossier::counts).unwrap_or_else(|| json!({}));
        o.show_widget("savoir_suivi", json!({
            "slug": slug, "sujet": sujet, "minutes": minutes, "axe": job.axe, "debut": js::now_ms(), "phase": "attente", "deja": known,
            "fichier": job.json_file.to_string_lossy()
        }));
        self.changed();
        let creating = job.axe.is_empty() && existing.is_none();
        let w = self.o.clone();
        js::spawn(async move {
            if let Some(o) = w.upgrade() {
                o.savoir.launch(job, existing).await;
            }
        });
        let say = if creating {
            format!("Je prépare le dossier sur {sujet}, pendant {minutes} minutes environ. Je te préviens quand il est prêt.")
        } else {
            format!("Je complète le dossier sur {sujet}, pendant {minutes} minutes environ.")
        };
        json!({ "ok": true, "slug": slug, "say": say, "display": format!("Dossier : {sujet}") })
    }

    /// Arrête la recherche au-delà du budget (+ 2 min) et garde ce qui a déjà été écrit. Une tâche disparue
    /// (fermée par l'utilisateur) est traitée de même.
    fn watch(&self, text: String) {
        let w = self.o.clone();
        js::spawn(async move {
            let mut seen_running = false;
            let mut stamp: Option<(u64, std::time::SystemTime)> = None;
            let mut md_len = 0u64;
            let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
            loop {
                js::sleep_ms(WATCH_MS).await;
                let Some(o) = w.upgrade() else { return };
                let Some(job) = o.savoir.pending.borrow().get(&text).cloned() else { return };
                let running = o.running.borrow().iter().find(|(_, t)| t.text == text).map(|(id, t)| (id.clone(), t.started_at));
                let queued = o.queue.borrow().iter().any(|q| q.text == text);
                // Fichier du dossier réécrit par l'IA : chiffres et éléments nouveaux vers la card de suivi.
                let meta = std::fs::metadata(&job.json_file).ok().and_then(|m| Some((m.len(), m.modified().ok()?)));
                let md = std::fs::metadata(&job.md_file).map(|m| m.len()).unwrap_or(0);
                if meta.is_some() && meta != stamp {
                    stamp = meta;
                    let parsed = std::fs::read_to_string(&job.json_file).ok().and_then(|t| dossier::parse_loose(&t)).map(|v| dossier::validate(&v));
                    match parsed {
                        Some(Ok(d)) => {
                            let fresh: Vec<J> = dossier::items_of(&d)
                                .into_iter()
                                .filter(|(k, _, _)| known.insert(k.clone()))
                                .take(60)
                                .map(|(_, kind, t)| json!({ "type": kind, "texte": js::slice_chars(&t, 160) }))
                                .collect();
                            o.savoir.progress(&job.slug, json!({ "phase": "recherche", "taskId": running.as_ref().map(|r| r.0.clone()), "compte": dossier::counts(&d), "nouveaux": fresh, "rapport": md }));
                        }
                        Some(Err(e)) => o.savoir.progress(&job.slug, json!({ "phase": "recherche", "taskId": running.as_ref().map(|r| r.0.clone()), "note": format!("fichier en cours d'écriture ({e})") })),
                        None => o.savoir.progress(&job.slug, json!({ "phase": "recherche", "taskId": running.as_ref().map(|r| r.0.clone()), "note": "fichier du dossier pas encore lisible" })),
                    }
                } else if md != md_len || (running.is_some() && !seen_running) {
                    o.savoir.progress(&job.slug, json!({ "phase": if running.is_some() { "recherche" } else { "attente" }, "taskId": running.as_ref().map(|r| r.0.clone()), "rapport": md }));
                }
                md_len = md;
                match running {
                    Some((id, started)) => {
                        seen_running = true;
                        if js::now_ms() - started > job.minutes as i64 * 60_000 + GRACE_MS {
                            o.savoir.pending.borrow_mut().remove(&text);
                            o.savoir.save_jobs();
                            (o.savoir.log)(format!("Savoir : temps écoulé pour {}, la recherche s'arrête", job.slug));
                            o.cancel(&id);
                            o.savoir.finish(job, String::new(), true, true).await;
                            return;
                        }
                    }
                    None if queued || !seen_running => {}
                    None => {
                        o.savoir.pending.borrow_mut().remove(&text);
                        o.savoir.save_jobs();
                        o.savoir.finish(job, String::new(), true, true).await;
                        return;
                    }
                }
            }
        });
    }

    /// Fin de la tâche de fond : true si c'était une recherche « Savoir ».
    pub fn done(&self, text: &str, answer: &str, ok: bool) -> bool {
        let Some(job) = self.pending.borrow_mut().remove(text) else { return false };
        self.save_jobs();
        let w = self.o.clone();
        let answer = answer.to_string();
        js::spawn(async move {
            if let Some(o) = w.upgrade() {
                o.savoir.finish(job, answer, ok, false).await;
            }
        });
        true
    }

    async fn finish(&self, job: Job, answer: String, ok: bool, stopped: bool) {
        let o = self.o.up();
        let slug = job.slug.clone();
        self.progress(&slug, json!({ "phase": "verification", "note": if stopped { "temps écoulé : la recherche s'arrête, ce qui est écrit est gardé" } else { "recherche terminée : vérification du dossier écrit par l'IA" } }));
        let raw = std::fs::read_to_string(&job.json_file).ok().and_then(|t| dossier::parse_loose(&t));
        let parsed = match raw {
            Some(v) => dossier::validate(&v),
            None if ok && !stopped => Err("l'IA n'a pas écrit le fichier du dossier".into()),
            None => Err("recherche arrêtée avant d'écrire le dossier".into()),
        };
        let had_content = self.get(&slug).map(|d| d["sections"].is_array() || d["fiche"].is_array()).unwrap_or(false);
        let (added, error) = match &parsed {
            Ok(add) => {
                let mut all = self.dossiers.borrow_mut();
                let d = all.entry(slug.clone()).or_insert_with(|| json!({ "meta": {} }));
                if js::gs(d, "sujet").is_empty() {
                    d["sujet"] = json!(job.sujet);
                }
                (dossier::merge(d, add), String::new())
            }
            Err(e) => (0, e.clone()),
        };
        // Rapport Markdown : copié à côté du dossier.
        if let Ok(md) = std::fs::read_to_string(&job.md_file) {
            if md.trim().chars().count() > 40 {
                let dir = self.dir.join(&slug).join("rapports");
                let _ = std::fs::create_dir_all(&dir);
                let name = job.md_file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "rapport.md".into());
                let _ = std::fs::write(dir.join(name), js::slice_chars(&md, 400_000));
            }
        }
        if let Ok(add) = &parsed {
            self.progress(&slug, json!({ "phase": "verification", "compte": dossier::counts(add), "ajouts": added, "note": format!("dossier valide : {added} élément(s) nouveau(x) fusionné(s), doublons écartés") }));
        } else {
            self.progress(&slug, json!({ "phase": "verification", "erreur": error }));
        }
        let (images, glb) = if parsed.is_ok() { self.download_media(&slug).await } else { (0, 0) };
        let etat = if parsed.is_ok() || had_content { "pret" } else { "echec" };
        {
            let mut all = self.dossiers.borrow_mut();
            if let Some(d) = all.get_mut(&slug) {
                let m = &mut d["meta"];
                m["etat"] = json!(etat);
                m["maj"] = json!(js::iso_now());
                if !m["recherches"].is_array() {
                    m["recherches"] = json!([]);
                }
                m["recherches"].as_array_mut().unwrap().push(json!({
                    "date": js::iso_now(), "axe": job.axe, "minutes": job.minutes, "ajouts": added, "images": images, "modeles": glb,
                    "statut": if parsed.is_err() { "echec" } else if stopped { "temps_ecoule" } else { "ok" }, "erreur": error
                }));
            }
        }
        self.save(&slug);
        let passages = self.passages(&slug);
        self.progress(&slug, json!({ "phase": "indexation", "note": format!("{} passage(s) à indexer dans le RAG du dossier", passages.len()) }));
        let computed = self.index(&slug).ensure(&passages).await.unwrap_or(0);
        let vectors = self.index(&slug).vector_count();
        let ready = if etat == "pret" {
            self.progress(&slug, json!({ "phase": "presentation", "note": "préparation de la présentation selon le profil du dossier (et recherche d'un modèle 3D réaliste pour un objet)" }));
            self.prepare(&slug).await
        } else {
            json!({})
        };
        let total = self.get(&slug).map(|d| dossier::counts(&d)).unwrap_or_else(|| json!({}));
        self.progress(&slug, json!({
            "phase": if etat == "pret" { "pret" } else { "echec" }, "fin": true, "tempsEcoule": stopped, "erreur": error,
            "rag": { "passages": passages.len(), "vecteurs": vectors, "calcules": computed, "semantique": self.index(&slug).model_installed() },
            "ajouts": added, "images": images, "modeles": glb, "total": total, "preparation": ready,
            "exemples": passages.iter().rev().take(3).map(|p| js::slice_chars(&p.text, 180)).collect::<Vec<_>>()
        }));
        (self.log)(format!("Savoir : {slug} — {added} ajout(s), {images} image(s), {glb} modèle(s), {} passage(s), {computed} vecteur(s) calculé(s){}", passages.len(), if error.is_empty() { String::new() } else { format!(", erreur : {error}") }));
        o.record("savoir_fin", json!({ "dossier": slug, "ajouts": added, "images": images, "passages": passages.len(), "statut": etat, "tempsEcoule": stopped }));
        self.changed();
        let sujet = job.sujet.clone();
        let text = if parsed.is_err() {
            if had_content { format!("La recherche complémentaire sur {sujet} n'a rien apporté : {error}.") } else { format!("Le dossier sur {sujet} n'a pas pu être constitué : {error}.") }
        } else {
            let spoken = crate::re!(r"[#*_`>]").replace_all(&answer, "");
            let spoken = js::slice_chars(crate::re!(r"\s+").replace_all(&spoken, " ").trim(), 400);
            let extra = match ready["finale"].as_str() {
                Some("exploration") => format!(" La présentation est prête, avec un modèle 3D{}.", if ready["modeles3d"].as_u64().unwrap_or(0) > 0 { " réaliste" } else { "" }),
                Some("media") => " La présentation est prête, avec de quoi écouter et regarder.".to_string(),
                Some(_) => " La présentation est prête.".to_string(),
                None => String::new(),
            };
            format!(
                "Le dossier sur {sujet} est prêt{} : {added} éléments ajoutés, {images} images. {spoken}{extra}",
                if stopped { ", le temps imparti est écoulé" } else { "" }
            )
        };
        o.notify(json!({
            "text": text,
            "title": format!("Dossier · {sujet}"),
            "source": "savoir",
            "rows": [
                { "icon": "▶", "title": "Présenter", "sub": "présentation animée", "say": format!("Présente-moi {sujet}") },
                { "icon": "◈", "title": "Ouvrir le dossier", "sub": "card Data", "say": format!("Ouvre le dossier {sujet}") }
            ]
        }));
    }

    /// Images et modèle 3D du dossier pas encore téléchargés. Rend (images, modèles) récupérés.
    async fn download_media(&self, slug: &str) -> (usize, usize) {
        let dir = self.dir.join(slug);
        let _ = std::fs::create_dir_all(dir.join("images"));
        let mut size = dir_size(&dir);
        let todo: Vec<(usize, String)> = self.get(slug).and_then(|d| d["images"].as_array().cloned()).unwrap_or_default().iter().enumerate().filter(|(_, i)| js::gs(i, "fichier").is_empty() && !js::gb(i, "echec")).map(|(n, i)| (n, js::gs(i, "url"))).collect();
        let mut got = 0;
        let total = todo.len();
        if total > 0 {
            self.progress(slug, json!({ "phase": "images", "fait": 0, "sur": total }));
        }
        for (k, (n, url)) in todo.into_iter().enumerate() {
            if size > DOSSIER_MAX_BYTES {
                break;
            }
            let res = download_image(&url, false).await;
            let legende = self.get(slug).map(|d| js::gs(&d["images"][n], "legende")).unwrap_or_default();
            self.progress(slug, json!({ "phase": "images", "fait": k + 1, "sur": total, "image": { "legende": legende, "ok": res.is_ok(), "erreur": res.as_ref().err() } }));
            let mut all = self.dossiers.borrow_mut();
            let Some(img) = all.get_mut(slug).and_then(|d| d["images"].get_mut(n)) else { continue };
            match res {
                Ok((bytes, ext)) => {
                    let name = format!("{}.{ext}", hash(&url));
                    if std::fs::write(dir.join("images").join(&name), &bytes).is_ok() {
                        size += bytes.len() as u64;
                        img["fichier"] = json!(name);
                        got += 1;
                    }
                }
                Err(e) => {
                    img["echec"] = json!(true);
                    img["erreur"] = json!(e);
                }
            }
        }
        let models: Vec<(usize, String)> = self.get(slug).and_then(|d| d["modeles3d"].as_array().cloned()).unwrap_or_default().iter().enumerate().filter(|(_, m)| !js::gs(m, "url_glb").is_empty() && js::gs(m, "fichier").is_empty() && !js::gb(m, "echec")).map(|(n, m)| (n, js::gs(m, "url_glb"))).collect();
        let mut glb = 0;
        for (n, url) in models {
            if size > DOSSIER_MAX_BYTES {
                break;
            }
            self.progress(slug, json!({ "phase": "images", "note": format!("téléchargement du modèle 3D {url}") }));
            let res = download_glb(&url).await;
            let mut all = self.dossiers.borrow_mut();
            let Some(m) = all.get_mut(slug).and_then(|d| d["modeles3d"].get_mut(n)) else { continue };
            match res {
                Ok(bytes) => {
                    let name = format!("modele-{}.glb", hash(&url));
                    if std::fs::write(dir.join(&name), &bytes).is_ok() {
                        size += bytes.len() as u64;
                        m["fichier"] = json!(name);
                        glb += 1;
                    }
                }
                Err(e) => {
                    m["echec"] = json!(true);
                    m["erreur"] = json!(e);
                }
            }
        }
        (got, glb)
    }

    // --- Consultation ------------------------------------------------------------------------------

    /// Passages les plus proches de la question, dans un dossier ou dans tous.
    pub async fn search(&self, query: &str, slug: Option<&str>, limit: usize) -> Vec<J> {
        let slugs: Vec<String> = match slug {
            Some(s) => vec![s.to_string()],
            None => self.dossiers.borrow().keys().cloned().collect(),
        };
        let mut rows: Vec<(f32, J)> = vec![];
        for s in slugs {
            let passages = self.passages(&s);
            if passages.is_empty() {
                continue;
            }
            let hits = self.index(&s).search(query, &passages, limit).await;
            let by_id: HashMap<&str, &Passage> = passages.iter().map(|p| (p.element.as_str(), p)).collect();
            let sujet = self.get(&s).map(|d| js::gs(&d, "sujet")).unwrap_or_default();
            for h in hits {
                if let Some(p) = by_id.get(h.element.as_str()) {
                    rows.push((h.score, json!({ "dossier": s, "sujet": sujet, "extrait": js::slice_chars(&p.text, 1200), "score": (h.score * 100.0).round() / 100.0 })));
                }
            }
        }
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        rows.into_iter().take(limit).map(|(_, r)| r).collect()
    }

    /// Texte renvoyé à l'IA par savoir_chercher.
    pub async fn ai_search_text(&self, query: &str, dossier_name: &str, limit: usize) -> String {
        if self.dossiers.borrow().is_empty() {
            return "Aucun dossier Data : l'utilisateur peut en demander un (« fais un dossier sur… »).".into();
        }
        let slug = if dossier_name.trim().is_empty() { None } else { self.find(dossier_name) };
        if !dossier_name.trim().is_empty() && slug.is_none() {
            let names: Vec<String> = self.dossiers.borrow().values().map(|d| js::gs(d, "sujet")).collect();
            return format!("Aucun dossier ne s'appelle « {dossier_name} ». Dossiers : {}.", names.join(", "));
        }
        if let Some(s) = &slug {
            if let Some(d) = self.dossiers.borrow_mut().get_mut(s) {
                d["meta"]["questions"] = json!(d["meta"]["questions"].as_i64().unwrap_or(0) + 1);
            }
            self.save(s);
        }
        let rows = self.search(query, slug.as_deref(), limit).await;
        if rows.is_empty() {
            return "Aucun passage des dossiers Data ne correspond.".into();
        }
        rows.iter().map(|r| format!("### {} (dossier {})\n{}", js::gs(r, "sujet"), js::gs(r, "dossier"), js::gs(r, "extrait"))).collect::<Vec<_>>().join("\n\n")
    }

    // --- Card « Data », statistiques, suppression --------------------------------------------------------

    fn summary(&self, slug: &str, d: &J) -> J {
        let profil = self.profile(&js::gs(&d["meta"], "profil"));
        let count = |k: &str| d[k].as_array().map(|a| a.len()).unwrap_or(0);
        let images_ok = d["images"].as_array().map(|a| a.iter().filter(|i| !js::gs(i, "fichier").is_empty()).count()).unwrap_or(0);
        let vignette = d["images"].as_array().and_then(|a| {
            let ok: Vec<&J> = a.iter().filter(|i| !js::gs(i, "fichier").is_empty()).collect();
            ok.iter().find(|i| js::gs(i, "vue") == "profil").or(ok.first()).map(|i| js::gs(i, "fichier"))
        });
        json!({
            "slug": slug, "sujet": d["sujet"], "categorie": d["categorie"], "etat": if self.is_running(slug) { json!("en_cours") } else { d["meta"]["etat"].clone() },
            "resume": js::slice_chars(&js::gs(d, "resume"), 260), "vignette": vignette,
            "stats": {
                "sources": count("sources"), "sections": count("sections"), "fiche": count("fiche"), "images": images_ok, "imagesTrouvees": count("images"),
                "videos": count("videos"), "pieces": count("pieces"), "modeles": d["modeles3d"].as_array().map(|a| a.iter().filter(|m| !js::gs(m, "fichier").is_empty()).count()).unwrap_or(0),
                "maquette": !d["maquette"].is_null(), "explorable": explore::has_3d(d), "medias": explore::has_media(d), "questions": d["meta"]["questions"].as_i64().unwrap_or(0), "presentations": d["meta"]["presentations"].as_i64().unwrap_or(0),
                "recherches": d["meta"]["recherches"].as_array().map(|a| a.len()).unwrap_or(0)
            },
            "cree": d["meta"]["cree"], "maj": d["meta"]["maj"],
            "profil": { "id": profil["id"], "nom": profil["nom"], "icone": profil["icone"], "finale": profil["finale"], "troisD": profil["troisD"], "boutons": profils::buttons(&profil, &js::gs(d, "sujet")) }
        })
    }

    pub fn list(&self) -> Vec<J> {
        let mut rows: Vec<J> = self.dossiers.borrow().iter().map(|(s, d)| self.summary(s, d)).collect();
        rows.sort_by(|a, b| js::gs(b, "maj").cmp(&js::gs(a, "maj")).then(js::gs(b, "cree").cmp(&js::gs(a, "cree"))));
        rows
    }

    /// Fiche complète pour la card (sans le texte des sections, seulement leurs titres).
    pub fn detail(&self, slug: &str) -> Option<J> {
        let d = self.get(slug)?;
        let mut s = self.summary(slug, &d);
        s["resume"] = d["resume"].clone();
        s["fiche"] = d["fiche"].clone();
        s["sections"] = json!(d["sections"].as_array().map(|a| a.iter().map(|x| x["titre"].clone()).collect::<Vec<_>>()).unwrap_or_default());
        s["chronologie"] = d["chronologie"].clone();
        s["pieces"] = json!(d["pieces"].as_array().map(|a| a.iter().map(|p| json!({ "id": p["id"], "nom": p["nom"] })).collect::<Vec<_>>()).unwrap_or_default());
        s["images"] = json!(d["images"].as_array().map(|a| a.iter().filter(|i| !js::gs(i, "fichier").is_empty()).map(|i| json!({ "fichier": i["fichier"], "legende": i["legende"], "vue": i["vue"], "licence": i["licence"], "source": i["source"] })).collect::<Vec<_>>()).unwrap_or_default());
        s["videos"] = d["videos"].clone();
        s["sources"] = d["sources"].clone();
        s["recherches"] = d["meta"]["recherches"].clone();
        s["taille"] = json!(dir_size(&self.dir.join(slug)));
        s["passages"] = json!(self.passages(slug).len());
        s["vecteurs"] = json!(self.index(slug).vector_count());
        Some(s)
    }

    /// Fichier d'une image téléchargée d'un dossier (Jarvis Nomade l'envoie au site).
    pub fn image_path(&self, slug: &str, file: &str) -> Option<PathBuf> {
        if !crate::re!(r"^[0-9a-f]{16}\.(?:jpg|png|gif|webp)$").is_match(file) || !self.dossiers.borrow().contains_key(slug) {
            return None;
        }
        Some(self.dir.join(slug).join("images").join(file))
    }

    /// Image d'un dossier en adresse data: (la card ne lit jamais le disque elle-même).
    pub fn image(&self, slug: &str, file: &str) -> Option<String> {
        if !crate::re!(r"^[0-9a-f]{16}\.(?:jpg|png|gif|webp)$").is_match(file) || !self.dossiers.borrow().contains_key(slug) {
            return None;
        }
        let bytes = std::fs::read(self.dir.join(slug).join("images").join(file)).ok()?;
        let mime = match file.rsplit('.').next() {
            Some("png") => "image/png",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            _ => "image/jpeg",
        };
        Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    }

    pub fn delete(&self, slug: &str) -> bool {
        if self.presentation.borrow().as_ref().map(|p| p.slug == slug).unwrap_or(false) {
            self.end_presentation(false);
        }
        if self.exploring().as_deref() == Some(slug) {
            self.close_explore(false);
        }
        if self.is_running(slug) || self.dossiers.borrow_mut().remove(slug).is_none() {
            return false;
        }
        self.indexes.borrow_mut().remove(slug);
        let _ = std::fs::remove_dir_all(self.dir.join(slug));
        self.o.up().record("savoir_supprime", json!({ "dossier": slug }));
        self.changed();
        true
    }

    /// Card « Data » : la liste des dossiers, ou un dossier ouvert.
    pub fn show_list(&self) {
        self.o.up().show_widget("savoir", json!({ "view": "liste", "dossiers": self.list() }));
    }

    pub fn show(&self, slug: &str) -> bool {
        let Some(d) = self.detail(slug) else { return false };
        self.o.up().show_widget("savoir", json!({ "view": "dossier", "dossier": d }));
        true
    }

    /// Méthode « savoir » de la card (mêmes opérations que la voix).
    pub async fn dispatch(&self, op: &str, a: &J) -> J {
        let slug = js::gs(a, "slug");
        match op {
            "liste" => json!({ "ok": true, "dossiers": self.list() }),
            "ouvrir" => {
                if slug.is_empty() {
                    self.show_list();
                    json!({ "ok": true })
                } else {
                    json!({ "ok": self.show(&slug) })
                }
            }
            "detail" => self.detail(&slug).map(|d| json!({ "ok": true, "dossier": d })).unwrap_or_else(|| json!({ "ok": false })),
            "image" => json!({ "ok": true, "src": self.image(&slug, &js::gs(a, "fichier")) }),
            "creer" => {
                let sujet = js::slice_chars(js::gs(a, "sujet").trim(), 120);
                if sujet.chars().count() < 2 {
                    return json!({ "ok": false, "say": "Indique un sujet." });
                }
                let slug = self.find(&sujet).filter(|s| voice::same_subject(self, s, &sujet));
                let minutes = js::gn(a, "minutes").map(|n| n as u32).unwrap_or(DEFAULT_MINUTES);
                self.start(&sujet, slug.as_deref(), "", minutes)
            }
            "approfondir" | "lien" | "relancer" => {
                let Some(d) = self.get(&slug) else { return json!({ "ok": false, "say": "Dossier introuvable." }) };
                let axe = js::gs(a, if op == "lien" { "url" } else { "axe" });
                if op == "lien" && !dossier::url_ok(&axe, false) {
                    return json!({ "ok": false, "say": "Ce lien n'est pas une adresse web publique." });
                }
                let minutes = js::gn(a, "minutes").map(|n| n as u32).unwrap_or(if op == "lien" { 3 } else { DEFAULT_MINUTES });
                self.start(&js::gs(&d, "sujet"), Some(&slug), &axe, minutes)
            }
            "presenter" => self.present(&slug).await,
            "explorer" => self.explore(&slug, None).await,
            "medias" => self.open_media(&slug, None),
            "espace" => self.open_space(&slug).await,
            "ecouter" => self.listen(&js::gs(a, "requete")).await,
            "pieces3d" => {
                let nodes = js::str_list(&a["noeuds"]);
                json!({ "ok": true, "correspondance": self.model_pieces(&slug, &js::gs(a, "uid"), &nodes).await })
            }
            "explorer_ferme" => {
                self.close_explore(true);
                json!({ "ok": true })
            }
            "modele" => json!({ "ok": true, "src": self.model(&slug, &js::gs(a, "fichier")) }),
            "commande3d" => json!({ "ok": self.control3d(&js::gs(a, "action")) }),
            "scene" => self.scene(&js::gs(a, "id"), js::gn(a, "n").map(|n| n as i64).unwrap_or(0)),
            "fin" => {
                if self.presentation.borrow().as_ref().map(|p| p.id == js::gs(a, "id")).unwrap_or(false) {
                    self.end_presentation(js::gb(a, "complete"));
                }
                json!({ "ok": true })
            }
            "supprimer" => {
                let sujet = self.get(&slug).map(|d| js::gs(&d, "sujet")).unwrap_or_default();
                if self.delete(&slug) {
                    self.show_list();
                    json!({ "ok": true, "say": format!("Dossier {sujet} supprimé.") })
                } else {
                    json!({ "ok": false, "say": "Impossible de supprimer ce dossier (recherche en cours ?)." })
                }
            }
            _ => json!({ "ok": false, "error": "opération inconnue" }),
        }
    }

    /// Chiffres seulement (page d'aide).
    pub fn stats(&self) -> J {
        let all = self.dossiers.borrow();
        let images: usize = all.values().map(|d| d["images"].as_array().map(|a| a.iter().filter(|i| !js::gs(i, "fichier").is_empty()).count()).unwrap_or(0)).sum();
        let sources: usize = all.values().map(|d| d["sources"].as_array().map(|a| a.len()).unwrap_or(0)).sum();
        let slugs: Vec<String> = all.keys().cloned().collect();
        drop(all);
        let passages: usize = slugs.iter().map(|s| self.passages(s).len()).sum();
        let vecteurs: usize = slugs.iter().map(|s| self.index(s).vector_count()).sum();
        let installed = self.index("_").model_installed();
        self.indexes.borrow_mut().remove("_");
        json!({
            "dossiers": slugs.len(), "enCours": self.pending.borrow().len(), "passages": passages, "vecteurs": vecteurs, "images": images, "sources": sources,
            "taille": dir_size(&self.dir), "budgetParDefaut": DEFAULT_MINUTES, "budgetMax": MAX_MINUTES, "maxDossiers": MAX_DOSSIERS, "modeleInstalle": installed
        })
    }
}
