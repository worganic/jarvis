// Prise de notes (orchestrator/notes.js). « Prends des notes » : une card s'ouvre (titre, auteur, date) ;
// chaque phrase dite ensuite y est écrite jusqu'à « fin de la note ». Le texte reste modifiable dans la
// card. « Enregistre la note » écrit un fichier texte dans notes/ ; « liste mes notes », « ouvre la
// note 2 », « fais un résumé de la note » (IA, sans outil). Le noyau garde le texte de référence : les
// ajouts partent en « note_append », les modifications de la card reviennent par edit().

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use chrono::{Datelike, Local, TimeZone, Timelike};
use serde_json::json;

use super::automations::base36;
use super::intent::Intent;
use super::{Fut, OrchRef, Up};
use crate::js::{self, J};

const NUM: &str = r"(\d{1,3}|un|une|premiere|premier|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|onze|douze)";
const TOPIC: &str = "(?: (?:sur|pour|a propos de|a propos du|a propos des|au sujet de|au sujet du|concernant|intitulee?|appelee?|avec pour titre|titre|avec) (.+))?";
pub const MEETING: &str = "(?:reunion|visio|visioconference|conference|appel|call|discussion|conversation|meeting|entretien|point)";

fn lazy(cell: &'static std::sync::OnceLock<regex::Regex>, build: impl FnOnce() -> String) -> &'static regex::Regex {
    cell.get_or_init(|| regex::Regex::new(&build()).unwrap())
}

pub fn new_note_re() -> &'static regex::Regex {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    lazy(&R, || format!("^(?:(?:prends|prendre|prend|commence|demarre|lance|cree|creer|fais|fait|ecris)(?: moi)? (?:des |une |la |les |un )?(?:nouvelle )?(?:notes?|prise de notes?)|ouvre une nouvelle note|prends note|prise de notes?|nouvelle note|mode (?:note|notes|prise de notes?)){TOPIC}$"))
}
pub fn meeting_start_re() -> &'static regex::Regex {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    lazy(&R, || format!("^(?:(?:enregistre|enregistrer|transcris|transcrire|capture|note|prends en note|prends des notes de|mets par ecrit)(?: moi)? (?:la |cette |ma |notre |une |l |le |ce )?{MEETING}|(?:lance|demarre|commence|active)(?: moi)? (?:l )?(?:enregistrement|transcription) (?:de |d )?(?:la |cette |ma |notre |l |le |ce )?{MEETING}|mode {MEETING}){TOPIC}$"))
}
pub fn meeting_stop_re() -> &'static regex::Regex {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    lazy(&R, || format!("^(?:(?:arrete|stoppe|stop|termine|coupe|finis|fin de) (?:l )?(?:enregistrement|la transcription|transcription|d enregistrer|de transcrire)(?: de (?:la |l )?{MEETING})?|(?:fin|arrete|stoppe|termine) (?:de )?(?:la |l )?{MEETING}|la {MEETING} est (?:finie|terminee)|{MEETING} terminee?)$"))
}
pub fn save_re() -> &'static regex::Regex {
    crate::re!(r"^(?:enregistre|sauvegarde|sauve|garde|sauvegarder|enregistrer)(?: moi)? (?:la |cette |ma |les |mes )?(?:note|notes|prise de notes?|transcription|compte rendu)$")
}
fn stop_dictation_re() -> &'static regex::Regex {
    crate::re!(r"^(?:fin (?:de |des )?(?:la |les )?(?:note|notes|dictee|prise de notes?)|(?:arrete|stoppe|termine|stop|coupe|ferme) (?:la |les |de prendre des |de prendre les )?(?:note|notes|prise de notes?|dictee)|j ai fini|c est fini|c est tout|fin)$")
}
fn resume_dictation_re() -> &'static regex::Regex {
    crate::re!(r"^(?:reprends|continue|reprendre|continuer|reprends la prise de|continue la prise de)(?: moi)? (?:la |les |ma |mes )?(?:note|notes|dictee)$")
}
fn undo_phrase_re() -> &'static regex::Regex {
    crate::re!(r"^(?:efface|annule|supprime|retire) (?:ca|cela|la derniere phrase|le dernier passage|ce que je viens de dire|la derniere ligne)$")
}
pub fn list_re() -> &'static regex::Regex {
    crate::re!(r"^(?:(?:liste|affiche|montre|donne|lis|ouvre|lance|active|quelles sont)(?: moi)? (?:(?:la liste (?:de |des )?)?(?:mes|les|toutes les|toutes mes) notes(?: enregistrees| sauvegardees)?|(?:la |le )?(?:card|cards|carte|cartes|carde|cardes|module|modules|widget) (?:de |des |d |mes )?notes?(?: \d{1,3})?)|liste des notes|quelles notes (?:j ai|as tu|sont enregistrees)|mes notes)$")
}
fn open_num_re() -> &'static regex::Regex {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    lazy(&R, || format!("^(?:ouvre|affiche|montre|reprends|rouvre|lis|charge)(?: moi)? (?:la )?note (?:numero |n |no )?{NUM}$"))
}
fn open_title_re() -> &'static regex::Regex {
    crate::re!(r"^(?:ouvre|affiche|rouvre|charge)(?: moi)? (?:la )?note (?:sur |de |du |des |intitulee |appelee )?(.{3,})$")
}
fn show_current_re() -> &'static regex::Regex {
    crate::re!(r"^(?:montre|affiche|ouvre|rouvre|remontre)(?: moi)? (?:la |ma )?(?:note|prise de notes?)(?: en cours)?$")
}
pub fn summary_re() -> &'static regex::Regex {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    lazy(&R, || format!("^(?:(?:fais|fait|donne|ecris|redige|genere|prepare|faire)(?: moi)? (?:un |le |une )?(?:resume|compte rendu|synthese)|resume(?: moi)?)(?: (?:de |du |des |d )?(?:la |cette |ma |notre |l )?(?:note|notes|{MEETING}|prise de notes?)(?: (?:numero |n |no )?{NUM})?)?$"))
}

pub const SUMMARY_PROMPT: &str = "Tu résumes une note ou la transcription d’une réunion, en français. La transcription vient d’une reconnaissance vocale : corrige mentalement les mots mal reconnus, ignore les hésitations et les répétitions. Réponds en texte brut, sans markdown (pas de #, pas de **), avec cette structure : une première ligne « En bref : » suivie d’une seule phrase ; puis « Points clés : » avec des lignes commençant par « - » ; puis, s’il y en a, « Décisions : » et « Actions à faire : » (qui fait quoi, pour quand) avec des lignes « - ». N’invente rien : ce qui n’est pas dans le texte n’apparaît pas dans le résumé.";

fn number_of(spoken: &str) -> Option<i64> {
    let s = spoken.trim();
    if crate::re!(r"^\d{1,3}$").is_match(s) {
        return s.parse().ok();
    }
    Some(match s {
        "un" | "une" | "premiere" | "premier" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "onze" => 11,
        "douze" => 12,
        _ => return None,
    })
}

pub fn slug(text: &str) -> String {
    let t = js::fold_lower(if text.is_empty() { "note" } else { text });
    let t = crate::re!(r"[^a-z0-9]+").replace_all(&t, "-");
    let t = js::slice_chars(crate::re!(r"^-|-$").replace_all(&t, "").as_ref(), 50);
    if t.is_empty() { "note".into() } else { t }
}

fn short_date(ms: i64) -> String {
    js::local_of(ms).format("%d/%m/%Y à %H:%M").to_string()
}

pub fn long_date(ms: i64) -> String {
    let d = js::local_of(ms);
    format!("{} {} à {:02}:{:02}", js::fr_long_date(&d), d.year(), d.hour(), d.minute())
}

/// Titre dit tel quel (casse et accents d'origine) : « prends des notes sur le Budget 2027 ».
pub fn spoken_topic(raw: &str) -> String {
    match crate::re!(r"(?i)\s(?:sur|pour|à propos de|à propos du|à propos des|a propos de|au sujet de|au sujet du|concernant|intitulée?|appelée?|avec pour titre|titre|avec)\s+(.+?)[\s.!?]*$").captures(raw) {
        Some(m) => js::capitalize(m[1].trim()),
        None => String::new(),
    }
}

fn separator() -> String {
    "─".repeat(40)
}

fn basename(p: &Path) -> String {
    p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
}

#[derive(Clone)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub author: String,
    pub created: i64,
    pub kind: String,
    pub body: String,
    pub file: Option<PathBuf>,
    pub dirty: bool,
    pub saved_at: i64,
}

#[derive(Clone)]
pub struct NoteFile {
    pub file: PathBuf,
    pub title: String,
    pub author: String,
    pub date_label: String,
    pub kind: String,
    pub body: String,
    pub mtime: i64,
    pub words: usize,
}

pub struct Notes {
    o: OrchRef,
    pub dir: PathBuf,
    log: Rc<dyn Fn(String)>,
    pub current: RefCell<Option<Note>>,
    pub listening: Cell<bool>,
    last_phrase_at: Cell<i64>,
    last_append: RefCell<Option<(String, String)>>,
    last_list: RefCell<Vec<NoteFile>>,
    seq: Cell<u64>,
    summarizing: Cell<bool>,
}

impl Notes {
    pub fn new(o: OrchRef, dir: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        Self {
            o,
            dir,
            log,
            current: RefCell::new(None),
            listening: Cell::new(false),
            last_phrase_at: Cell::new(0),
            last_append: RefCell::new(None),
            last_list: RefCell::new(vec![]),
            seq: Cell::new(0),
            summarizing: Cell::new(false),
        }
    }

    // --- État et card ---------------------------------------------------------------------------

    pub fn author(&self) -> String {
        let o = self.o.up();
        let profile = o.config.get("profile");
        let name = js::gs(&profile, "name");
        let speaker = o.current_speaker.borrow().clone().unwrap_or_default();
        let mem = o.memory.get_user_name().unwrap_or_default();
        let n = [name, speaker, mem].into_iter().find(|s| !s.is_empty()).unwrap_or_else(|| "Moi".into());
        n.trim().to_string()
    }

    fn next_id(&self, at: i64) -> String {
        self.seq.set(self.seq.get() + 1);
        format!("note{}-{}", self.seq.get(), base36(at.max(0) as u64))
    }

    pub fn create(&self, title: &str, kind: &str) -> Note {
        let created = js::now_ms();
        let fallback = if kind == "reunion" { format!("Réunion du {}", short_date(created)) } else { format!("Note du {}", short_date(created)) };
        let n = Note {
            id: self.next_id(created),
            title: if title.is_empty() { fallback } else { title.to_string() },
            author: self.author(),
            created,
            kind: kind.to_string(),
            body: String::new(),
            file: None,
            dirty: false,
            saved_at: 0,
        };
        *self.current.borrow_mut() = Some(n.clone());
        *self.last_append.borrow_mut() = None;
        n
    }

    pub fn current_title(&self) -> String {
        self.current.borrow().as_ref().map(|n| n.title.clone()).unwrap_or_default()
    }

    fn card_data(&self) -> Option<J> {
        let c = self.current.borrow();
        let n = c.as_ref()?;
        Some(json!({
            "noteId": n.id, "title": n.title, "author": n.author, "dateLabel": long_date(n.created), "body": n.body,
            "noteKind": n.kind, "file": n.file.as_deref().map(basename).unwrap_or_default(), "dirty": n.dirty, "listening": self.listening.get()
        }))
    }

    pub fn show(&self) {
        if let Some(d) = self.card_data() {
            self.o.up().show_widget("note", d);
        }
    }

    fn publish_state(&self, extra: J) {
        let ev = {
            let c = self.current.borrow();
            let Some(n) = c.as_ref() else { return };
            let mut ev = json!({ "type": "note_state", "noteId": n.id, "listening": self.listening.get(), "dirty": n.dirty, "file": n.file.as_deref().map(basename).unwrap_or_default(), "title": n.title });
            js::assign(&mut ev, extra);
            ev
        };
        self.o.up().emit(ev);
    }

    pub fn set_listening(&self, on: bool) {
        let on = on && self.current.borrow().is_some();
        if on == self.listening.get() {
            return;
        }
        self.listening.set(on);
        self.last_phrase_at.set(js::now_ms());
        let o = self.o.up();
        o.emit(json!({ "type": "note_listen", "on": on }));
        o.record(if on { "note_dictee_debut" } else { "note_dictee_fin" }, json!({ "titre": self.current_title() }));
        self.publish_state(json!({}));
    }

    pub fn append(&self, text: &str, raw: bool) -> String {
        let (id, add) = {
            let mut c = self.current.borrow_mut();
            let Some(n) = c.as_mut() else { return String::new() };
            let mut add = if raw { text.to_string() } else { super::desktop::spoken_punctuation(text) };
            if add.trim().is_empty() {
                return String::new();
            }
            if !n.body.is_empty() && !n.body.ends_with(char::is_whitespace) && !crate::re!(r"^[\n.,;:!?…»]").is_match(&add) {
                add = format!(" {add}");
            }
            n.body.push_str(&add);
            n.dirty = true;
            (n.id.clone(), add)
        };
        *self.last_append.borrow_mut() = Some((id.clone(), add.clone()));
        self.o.up().emit(json!({ "type": "note_append", "noteId": id, "text": add }));
        add
    }

    /// Modification faite dans la card (texte complet).
    pub fn edit(&self, id: &str, patch: &J) -> J {
        let mut c = self.current.borrow_mut();
        let Some(n) = c.as_mut().filter(|n| n.id == id) else { return json!({ "ok": false, "error": "note fermée" }) };
        if let Some(body) = patch["body"].as_str() {
            if body != n.body {
                n.body = js::slice_chars(body, 2_000_000);
                n.dirty = true;
            }
        }
        if let Some(title) = patch["title"].as_str() {
            let t = title.trim();
            if !t.is_empty() && t != n.title {
                n.title = js::slice_chars(t, 200);
                n.dirty = true;
            }
        }
        json!({ "ok": true })
    }

    pub fn card_closed(&self) {
        self.set_listening(false);
    }

    // --- Fichiers -------------------------------------------------------------------------------

    fn file_text(n: &Note) -> String {
        let kind = if n.kind == "reunion" { "Réunion enregistrée (micro et son du PC)" } else { "Note" };
        [
            format!("Titre : {}", n.title),
            format!("Auteur : {}", n.author),
            format!("Date : {}", long_date(n.created)),
            format!("Type : {kind}"),
            separator(),
            String::new(),
            n.body.replace("\r\n", "\n").trim_end().to_string(),
            String::new(),
        ]
        .join("\n")
    }

    pub fn save(&self, id: Option<&str>, patch: Option<&J>) -> J {
        let cur_id = match self.current.borrow().as_ref() {
            Some(n) if id.map(|i| i == n.id).unwrap_or(true) => n.id.clone(),
            _ => return json!({ "ok": false, "say": "Aucune note ouverte." }),
        };
        if let Some(p) = patch {
            self.edit(&cur_id, p);
        }
        self.write_current()
    }

    fn write_current(&self) -> J {
        let res = {
            let mut c = self.current.borrow_mut();
            let Some(n) = c.as_mut() else { return json!({ "ok": false, "say": "Aucune note ouverte." }) };
            if n.body.trim().is_empty() {
                return json!({ "ok": false, "say": "La note est vide : rien à enregistrer." });
            }
            let write = (|| -> std::io::Result<PathBuf> {
                std::fs::create_dir_all(&self.dir)?;
                if n.file.is_none() {
                    n.file = Some(self.free_file(n.created, &n.title));
                }
                let file = n.file.clone().unwrap();
                std::fs::write(&file, Self::file_text(n))?;
                Ok(file)
            })();
            match write {
                Err(e) => Err(e.to_string()),
                Ok(file) => {
                    n.dirty = false;
                    n.saved_at = js::now_ms();
                    Ok((file, n.title.clone(), n.body.chars().count(), n.kind.clone(), n.saved_at))
                }
            }
        };
        match res {
            Err(e) => {
                (self.log)(format!("notes : enregistrement impossible ({e})"));
                json!({ "ok": false, "say": format!("Je n'ai pas pu enregistrer la note : {e}") })
            }
            Ok((file, title, chars, kind, saved_at)) => {
                self.o.up().record("note_enregistree", json!({ "titre": title, "fichier": file.to_string_lossy(), "caracteres": chars, "type": kind }));
                self.publish_state(json!({ "savedAt": saved_at }));
                json!({ "ok": true, "file": file.to_string_lossy(), "say": format!("Note enregistrée : {}.", basename(&file)) })
            }
        }
    }

    /// Nom de fichier libre pour une nouvelle note : « AAAA-MM-JJ_HHhMM_<titre>.txt » (suffixe -2, -3… si pris).
    fn free_file(&self, created: i64, title: &str) -> PathBuf {
        let stamp = js::local_of(created).format("%Y-%m-%d_%Hh%M").to_string();
        let mut file = self.dir.join(format!("{stamp}_{}.txt", slug(title)));
        let mut i = 2;
        while file.exists() {
            file = self.dir.join(format!("{stamp}_{}-{i}.txt", slug(title)));
            i += 1;
        }
        file
    }

    /// Note écrite ailleurs (site Nomade) : enregistrée comme les autres, sans toucher à la note ouverte.
    pub fn write_new(&self, title: &str, body: &str, origin: &str) -> Result<PathBuf, String> {
        let created = js::now_ms();
        let title = title.trim();
        let n = Note {
            id: self.next_id(created),
            title: if title.is_empty() { format!("Note du {}", short_date(created)) } else { title.to_string() },
            author: self.author(),
            created,
            kind: "note".into(),
            body: body.trim().to_string(),
            file: None,
            dirty: false,
            saved_at: 0,
        };
        if n.body.is_empty() {
            return Err("La note est vide.".into());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let file = self.free_file(created, &n.title);
        std::fs::write(&file, Self::file_text(&n)).map_err(|e| e.to_string())?;
        self.o.up().record("note_enregistree", json!({ "titre": n.title, "fichier": file.to_string_lossy(), "caracteres": n.body.chars().count(), "type": "note", "origine": origin }));
        Ok(file)
    }

    /// Note non enregistrée remplacée par une autre (ou Jarvis qui s'arrête) : rien n'est perdu.
    pub fn save_if_dirty(&self) -> bool {
        let dirty = self.current.borrow().as_ref().map(|n| n.dirty && !n.body.trim().is_empty()).unwrap_or(false);
        dirty && js::gb(&self.save(None, None), "ok")
    }

    pub fn parse_file(file: &Path) -> Option<NoteFile> {
        let text = std::fs::read_to_string(file).ok()?;
        let text = text.replace("\r\n", "\n");
        let lines: Vec<&str> = text.split('\n').collect();
        let mut head = std::collections::HashMap::new();
        let mut i = 0;
        while i < lines.len().min(12) {
            if crate::re!(r"^─{5,}$").is_match(lines[i].trim()) {
                i += 1;
                break;
            }
            if let Some(m) = crate::re!(r"^(Titre|Auteur|Date|Type) : (.*)$").captures(lines[i]) {
                head.insert(m[1].to_lowercase(), m[2].trim().to_string());
            } else if lines[i].trim().is_empty() {
            } else {
                i = 0;
                break;
            }
            i += 1;
        }
        let i = i.min(lines.len());
        let body = lines[i..].join("\n");
        let body = body.trim_start_matches('\n').trim_end().to_string();
        let mtime = std::fs::metadata(file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let stem = file.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        Some(NoteFile {
            file: file.to_path_buf(),
            title: head.get("titre").cloned().unwrap_or(stem),
            author: head.get("auteur").cloned().unwrap_or_default(),
            date_label: head.get("date").cloned().unwrap_or_default(),
            kind: if crate::re!(r"(?i)réunion").is_match(head.get("type").map(|s| s.as_str()).unwrap_or("")) { "reunion".into() } else { "note".into() },
            words: body.split_whitespace().count(),
            body,
            mtime,
        })
    }

    pub fn list(&self) -> Vec<NoteFile> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<NoteFile> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x.to_string_lossy().eq_ignore_ascii_case("txt")).unwrap_or(false))
            .filter_map(|p| Self::parse_file(&p))
            .collect();
        out.sort_by_key(|n| std::cmp::Reverse(n.mtime));
        out
    }

    pub fn load(&self, file: &Path) -> bool {
        let Some(p) = Self::parse_file(file) else { return false };
        self.save_if_dirty();
        self.set_listening(false);
        let created = crate::re!(r"^(\d{4})-(\d{2})-(\d{2})_(\d{2})h(\d{2})")
            .captures(&basename(file))
            .and_then(|m| {
                let n = |i: usize| m[i].parse::<u32>().unwrap_or(0);
                Local.with_ymd_and_hms(n(1) as i32, n(2), n(3), n(4), n(5), 0).earliest().map(|d| d.timestamp_millis())
            })
            .unwrap_or(if p.mtime != 0 { p.mtime } else { js::now_ms() });
        let author = if p.author.is_empty() { self.author() } else { p.author.clone() };
        *self.current.borrow_mut() = Some(Note {
            id: self.next_id(created),
            title: p.title,
            author,
            created,
            kind: p.kind,
            body: p.body,
            file: Some(file.to_path_buf()),
            dirty: false,
            saved_at: p.mtime,
        });
        *self.last_append.borrow_mut() = None;
        true
    }

    pub fn show_list(&self) -> Vec<NoteFile> {
        let all = self.list();
        let shown: Vec<NoteFile> = all.iter().take(40).cloned().collect();
        let rows: Vec<J> = shown
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let f = n.file.to_string_lossy().to_string();
                let words = format!("{} mot{}", n.words, if n.words > 1 { "s" } else { "" });
                let sub: Vec<String> = [n.date_label.clone(), words].into_iter().filter(|s| !s.is_empty()).collect();
                json!({
                    "icon": if n.kind == "reunion" { "◉" } else { "✎" },
                    "title": format!("{}. {}", i + 1, n.title),
                    "sub": sub.join(" · "),
                    "meta": if n.kind == "reunion" { "réunion" } else { "note" },
                    "call": { "method": "listAction", "args": ["note", f, "ouvrir"] },
                    "hint": "Clic : ouvrir cette note",
                    "say": format!("ouvre la note {}", i + 1),
                    "actions": [
                        { "icon": "▣", "title": "Ouvrir dans la card de note", "call": { "method": "listAction", "args": ["note", f, "ouvrir"] } },
                        { "icon": "✦", "title": "Résumé par l’IA", "call": { "method": "listAction", "args": ["note", f, "resumer"] } }
                    ]
                })
            })
            .collect();
        *self.last_list.borrow_mut() = shown;
        let count = rows.len();
        self.o.up().show_widget(
            "tool",
            json!({
                "title": "Mes notes", "tool": "notes", "rows": rows, "count": count, "ttl": 120000,
                "empty": "Aucune note enregistrée. Dis « prends des notes ».",
                "footer": format!("{} · clic : ouvrir · « résume la note 2 »", self.dir.to_string_lossy())
            }),
        );
        all
    }

    fn pick(&self, n: i64) -> Option<NoteFile> {
        if self.last_list.borrow().is_empty() {
            *self.last_list.borrow_mut() = self.list().into_iter().take(40).collect();
        }
        if n < 1 {
            return None;
        }
        self.last_list.borrow().get(n as usize - 1).cloned()
    }

    // --- Résumé par l'IA ------------------------------------------------------------------------

    pub async fn claude_summary(&self, text: String) -> Result<String, String> {
        let claude = self.o.up().claude_path().ok_or("Claude Code introuvable")?;
        use tokio::io::AsyncWriteExt;
        let mut cmd = tokio::process::Command::new(&claude);
        cmd.args([
            "-p", "--output-format", "text", "--tools", "", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}",
            "--permission-mode", "manual", "--no-session-persistence", "--system-prompt", SUMMARY_PROMPT, "--model", "sonnet",
        ])
        .current_dir(crate::paths::workspace())
        .env("JARVIS_TASK_ID", "resume-note")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        // La note passe par l'entrée standard : rien de son contenu dans une ligne de commande.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes()).await;
        }
        let out = tokio::time::timeout(std::time::Duration::from_secs(180), child.wait_with_output()).await.map_err(|_| "délai dépassé (3 min)".to_string())?.map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !stdout.is_empty() {
            Ok(stdout)
        } else {
            let err = js::slice_chars(String::from_utf8_lossy(&out.stderr).trim(), 300);
            Err(if err.is_empty() { format!("code {}", out.status.code().unwrap_or(-1)) } else { err })
        }
    }

    pub async fn summarize(&self) -> J {
        let Some(n) = self.current.borrow().clone() else { return json!({ "ok": false, "say": "La note est vide : rien à résumer." }) };
        if n.body.trim().is_empty() {
            return json!({ "ok": false, "say": "La note est vide : rien à résumer." });
        }
        if self.summarizing.get() {
            return json!({ "ok": false, "say": "Le résumé est déjà en cours." });
        }
        self.summarizing.set(true);
        self.publish_state(json!({ "summarizing": true }));
        let o = self.o.up();
        let task_id = format!("resume{}", base36(js::now_ms() as u64));
        o.emit(json!({ "type": "create", "id": task_id, "label": "Résumé de la note", "detail": js::slice_chars(&format!("Sonnet · {}", n.title), 60) }));
        // Le résumé précédent (s'il y en a un) n'est pas renvoyé à l'IA.
        let content = crate::re!(r"\n── Résumé \(IA").split(&n.body).next().unwrap_or("").trim().to_string();
        let kind = if n.kind == "reunion" { "transcription d’une réunion (« Participants » = les autres personnes de la visio)" } else { "note" };
        let input = format!("Titre : {}\nAuteur : {}\nDate : {}\nType : {kind}\n\n{content}", n.title, n.author, long_date(n.created));
        let res = self.claude_summary(input).await;
        let out = match res {
            Err(e) => {
                (self.log)(format!("notes : résumé impossible ({e})"));
                o.emit(json!({ "type": "finish", "id": task_id, "status": "error" }));
                json!({ "ok": false, "say": format!("Je n'ai pas pu faire le résumé : {e}") })
            }
            Ok(summary) => {
                let summary = crate::re!(r"(?m)\*\*|^#+\s*").replace_all(&summary, "").trim().to_string();
                let same = self.current.borrow().as_ref().map(|c| c.id == n.id).unwrap_or(false);
                if !same {
                    json!({ "ok": false, "say": "La note a changé entre-temps : résumé abandonné." })
                } else {
                    let now = Local::now();
                    let sep = if n.body.trim().is_empty() { "" } else { "\n\n" };
                    self.append(&format!("{sep}── Résumé (IA, {}) ──\n{summary}\n", now.format("%d/%m %H:%M")), true);
                    o.record("note_resume", json!({ "titre": n.title, "caracteres": content.chars().count(), "resume": js::slice_chars(&summary, 2000) }));
                    o.emit(json!({ "type": "finish", "id": task_id, "status": "done" }));
                    let saved = if n.file.is_some() { js::gb(&self.save(None, None), "ok") } else { false };
                    let found = crate::re!(r"(?im)^En bref\s*:\s*(.+)$").captures(&summary).map(|m| m[1].trim().to_string()).unwrap_or_default();
                    let brief = if found.is_empty() { "Résumé prêt.".to_string() } else { js::capitalize(&found) };
                    json!({ "ok": true, "say": format!("{brief} Le résumé complet est ajouté à la note{}.", if saved { " et enregistré" } else { "" }) })
                }
            }
        };
        self.summarizing.set(false);
        self.publish_state(json!({ "summarizing": false }));
        out
    }

    // --- Voix ------------------------------------------------------------------------------------

    /// Phrase dite pendant la prise de notes : true si elle a été traitée ici (texte ou commande).
    pub async fn handle_voice(&self, clean: &str, wake: bool) -> bool {
        if !self.listening.get() || self.current.borrow().is_none() {
            return false;
        }
        if js::now_ms() - self.last_phrase_at.get() > 15 * 60 * 1000 {
            self.set_listening(false);
            return false;
        }
        let t = js::fold_lower(clean);
        let t = crate::re!(r"['’\-?!,;:.…]").replace_all(&t, " ");
        let t = crate::re!(r"\s+").replace_all(&t, " ").trim().to_string();
        let t = crate::re!(r"^jarvis ").replace(&t, "").into_owned();
        self.last_phrase_at.set(js::now_ms());
        let o = self.o.up();
        let say = |text: String| {
            o.emit(json!({ "type": "say", "text": text }));
            true
        };
        if stop_dictation_re().is_match(&t) {
            self.set_listening(false);
            let dirty = self.current.borrow().as_ref().map(|n| n.dirty).unwrap_or(false);
            return say(if dirty { "Prise de notes terminée. Clique sur Enregistrer, ou dis « enregistre la note ».".into() } else { "Prise de notes terminée.".into() });
        }
        if save_re().is_match(&t) {
            return say(js::gs(&self.save(None, None), "say"));
        }
        if undo_phrase_re().is_match(&t) {
            let last = self.last_append.borrow().clone();
            let replaced = {
                let mut c = self.current.borrow_mut();
                let n = c.as_mut().unwrap();
                match last {
                    Some((id, text)) if id == n.id && n.body.ends_with(&text) => {
                        n.body.truncate(n.body.len() - text.len());
                        Some((n.id.clone(), n.body.clone()))
                    }
                    _ => None,
                }
            };
            let Some((id, body)) = replaced else { return say("Rien à effacer.".into()) };
            *self.last_append.borrow_mut() = None;
            o.emit(json!({ "type": "note_replace", "noteId": id, "body": body }));
            return say("Effacé.".into());
        }
        // « Jarvis, … » : commande (notes ou autre intention locale), pas du texte à écrire.
        if wake {
            let intent = self.match_(&t, clean).or_else(|| o.router.route(clean, true));
            if intent.map(|i| !i.ignored).unwrap_or(false) {
                return false;
            }
            if !crate::re!(r"^(?:note|ecris|ajoute)\b").is_match(&t) {
                return false;
            }
        }
        o.usage_daily.add_local("local");
        let text = if wake { crate::re!(r"(?i)^(?:note|écris|ecris|ajoute)(?:\s+(?:que|ça|ca|ceci))?\s*[:,]?\s+").replace(clean, "").into_owned() } else { clean.to_string() };
        self.append(&text, false);
        true
    }

    // --- Intentions du routeur ---------------------------------------------------------------------

    fn intent<F, Fu>(&self, kind: &str, label: &str, detail: &str, f: F) -> Intent
    where
        F: Fn(Rc<super::Orchestrator>) -> Fu + 'static,
        Fu: std::future::Future<Output = J> + 'static,
    {
        let o = self.o.clone();
        let f = Rc::new(f);
        Intent::new(kind, label, detail, move || {
            let (o, f) = (o.clone(), f.clone());
            async move { f(o.up()).await }
        })
    }

    pub fn match_(&self, t: &str, raw: &str) -> Option<Intent> {
        let has_current = self.current.borrow().is_some();
        if new_note_re().is_match(t) {
            let title = spoken_topic(raw);
            let detail = if title.is_empty() { "nouvelle note".to_string() } else { title.clone() };
            return Some(self.intent("note_new", "Prise de notes", &detail, move |o| {
                let title = title.clone();
                async move {
                    let n = &o.notes;
                    let had_dirty = n.save_if_dirty();
                    n.create(&title, "note");
                    n.show();
                    n.set_listening(true);
                    json!({ "say": format!("{}Je t’écoute : tout ce que tu dis est écrit dans la note. Dis « fin de la note » pour arrêter.", if had_dirty { "Note précédente enregistrée. " } else { "" }), "display": n.current_title() })
                }
            }));
        }
        if save_re().is_match(t) && has_current {
            return Some(self.intent("note_save", "Note", "enregistrement", |o| async move {
                let r = o.notes.save(None, None);
                let display = if js::gb(&r, "ok") { Path::new(&js::gs(&r, "file")).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default() } else { js::gs(&r, "say") };
                json!({ "say": r["say"], "display": display })
            }));
        }
        if resume_dictation_re().is_match(t) && has_current {
            return Some(self.intent("note_resume_dictation", "Prise de notes", &self.current_title(), |o| async move {
                o.notes.show();
                o.notes.set_listening(true);
                json!({ "say": "Je reprends la prise de notes.", "display": o.notes.current_title() })
            }));
        }
        if list_re().is_match(t) {
            return Some(self.intent("notes_list", "Mes notes", "liste", |o| async move {
                let all = o.notes.show_list();
                let n = all.len();
                json!({
                    "say": if n > 0 { format!("Tu as {n} note{}. La plus récente : {}.", if n > 1 { "s" } else { "" }, all[0].title) } else { "Tu n’as encore aucune note enregistrée.".into() },
                    "display": format!("{n} notes")
                })
            }));
        }
        if let Some(m) = summary_re().captures(t) {
            if m.get(1).is_some() || has_current || crate::re!(r"\b(?:note|notes|reunion|visio)\b").is_match(t) {
                let n = m.get(1).and_then(|x| number_of(x.as_str()));
                let detail = n.map(|n| format!("note {n}")).unwrap_or_else(|| "note en cours".into());
                return Some(self.intent("note_summary", "Résumé IA", &detail, move |o| async move {
                    let notes = &o.notes;
                    if let Some(n) = n {
                        let Some(p) = notes.pick(n) else { return json!({ "say": format!("Il n'y a pas de note {n}. Dis « liste mes notes »."), "display": "Note introuvable" }) };
                        notes.load(&p.file);
                        notes.show();
                    } else if notes.current.borrow().is_none() {
                        return json!({ "say": "Aucune note ouverte. Dis « liste mes notes », puis « résume la note 2 ».", "display": "Aucune note" });
                    }
                    o.emit(json!({ "type": "say", "text": "Je prépare le résumé." }));
                    let r = notes.summarize().await;
                    json!({ "say": r["say"], "display": if js::gb(&r, "ok") { json!("Résumé ajouté à la note") } else { r["say"].clone() } })
                }));
            }
        }
        if let Some(m) = open_num_re().captures(t) {
            let n = number_of(&m[1]).unwrap_or(0);
            return Some(self.intent("note_open", "Note", &format!("n° {n}"), move |o| async move {
                let Some(p) = o.notes.pick(n) else { return json!({ "say": format!("Il n'y a pas de note {n}."), "display": "Note introuvable" }) };
                o.notes.load(&p.file);
                o.notes.show();
                json!({ "say": format!("Note {}.", p.title), "display": p.title })
            }));
        }
        if show_current_re().is_match(t) {
            return Some(self.intent("note_show", "Note", "en cours", |o| async move {
                let notes = &o.notes;
                if notes.current.borrow().is_none() {
                    let Some(last) = notes.list().into_iter().next() else { return json!({ "say": "Aucune note pour l’instant. Dis « prends des notes ».", "display": "Aucune note" }) };
                    notes.load(&last.file);
                }
                notes.show();
                json!({ "say": null, "display": notes.current_title() })
            }));
        }
        if let Some(m) = open_title_re().captures(t) {
            let wanted = m[1].to_string();
            if !crate::re!(r"^(?:numero|n|no) ").is_match(&wanted) {
                let fold = |s: &str| crate::re!(r"[^a-z0-9]+").replace_all(&js::fold_lower(s), " ").trim().to_string();
                let all = self.list();
                let words: Vec<&str> = wanted.split(' ').filter(|w| w.chars().count() > 3).collect();
                let found = all.iter().find(|p| fold(&p.title).contains(&wanted)).or_else(|| all.iter().find(|p| words.iter().all(|w| fold(&p.title).contains(w)))).cloned();
                if let Some(found) = found {
                    let title = found.title.clone();
                    return Some(self.intent("note_open", "Note", &title, move |o| {
                        let found = found.clone();
                        async move {
                            o.notes.load(&found.file);
                            o.notes.show();
                            json!({ "say": format!("Note {}.", found.title), "display": found.title })
                        }
                    }));
                }
            }
        }
        None
    }

    /// Boutons de la card.
    pub fn action(&self, id: &str, action: &str, payload: &J) -> Fut<J> {
        let o = self.o.clone();
        let (id, action, payload) = (id.to_string(), action.to_string(), payload.clone());
        Box::pin(async move {
            let orch = o.up();
            let notes = &orch.notes;
            let say = |r: J| {
                if js::truthy(&r["say"]) {
                    orch.emit(json!({ "type": "say", "text": r["say"] }));
                }
                r
            };
            match action.as_str() {
                "new" => {
                    notes.save_if_dirty();
                    notes.create(&js::gs(&payload, "title"), "note");
                    notes.show();
                    notes.set_listening(true);
                    return json!({ "ok": true });
                }
                "list" => {
                    notes.show_list();
                    return json!({ "ok": true });
                }
                _ => {}
            }
            let cur = notes.current.borrow().as_ref().map(|n| n.id.clone());
            let Some(cur) = cur.filter(|c| id.is_empty() || *c == id) else { return json!({ "ok": false, "say": "Cette note n’est plus ouverte." }) };
            if payload["body"].is_string() || payload["title"].is_string() {
                notes.edit(&cur, &payload);
            }
            match action.as_str() {
                "save" => say(notes.save(None, None)),
                "dictate" => {
                    notes.set_listening(!notes.listening.get());
                    json!({ "ok": true, "listening": notes.listening.get() })
                }
                "meeting_start" => say(orch.meetings.start(&json!({})).await),
                "summary" => say(notes.summarize().await),
                other => json!({ "ok": false, "error": format!("action inconnue : {other}") }),
            }
        })
    }

    pub fn stop(&self) {
        self.save_if_dirty();
    }
}
