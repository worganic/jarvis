// Jarvis Nomade — données partageables. Chaque source est lue dans son magasin (listes à fiches,
// listes, notes, rappels…) et mise sous une forme commune pour le site : `_t` (titre), `_s` (sous-titre),
// `_d` (détails [libellé, valeur]) et les champs propres au type. Rien n'est écrit ici.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::collections::schema;
use super::super::Orchestrator;
use crate::js::{self, J};

/// Taille maximale d'un texte envoyé (notes longues, transcriptions).
const MAX_BODY: usize = 20000;
/// Image locale envoyée au site : au-delà, seule l'adresse d'origine (s'il y en a une) est donnée.
pub const MAX_IMAGE_BYTES: u64 = 2_500_000;

thread_local! {
    /// Empreinte des images locales (fichier, date, taille) : un fichier n'est relu que s'il a changé.
    /// Nom publié (« <sha256>.<ext> ») → fichier, pour l'envoi.
    static IMAGES: RefCell<(HashMap<PathBuf, (u64, u64, String)>, HashMap<String, PathBuf>)> = RefCell::new((HashMap::new(), HashMap::new()));
    /// Sources lues de façon asynchrone (agenda Google, état git des projets) : relues avant chaque envoi
    /// (`refresh_live`), au plus toutes les LIVE_TTL_MS. Source → (instant, enregistrements).
    static LIVE: RefCell<HashMap<&'static str, (i64, Vec<(String, J)>)>> = RefCell::new(HashMap::new());
}

const LIVE_TTL_MS: i64 = 5 * 60000;
/// Agenda envoyé : de 7 jours en arrière à 90 jours devant.
const AGENDA_BACK_MS: i64 = 7 * 86_400_000;
const AGENDA_AHEAD_MS: i64 = 90 * 86_400_000;

fn live(id: &'static str) -> Option<Vec<(String, J)>> {
    LIVE.with(|l| l.borrow().get(id).map(|(_, rows)| rows.clone()))
}

fn live_fresh(id: &'static str) -> bool {
    LIVE.with(|l| l.borrow().get(id).map(|(at, _)| js::now_ms() - at < LIVE_TTL_MS).unwrap_or(false))
}

/// Relit l'agenda et les projets s'ils sont partagés (avant un envoi).
pub async fn refresh_live(o: &Orchestrator, shared: &[String]) {
    if shared.iter().any(|s| s == "agenda") && !live_fresh("agenda") {
        let now = js::now_ms();
        let events = o.google.agenda_periode(now - AGENDA_BACK_MS, now + AGENDA_AHEAD_MS).await;
        let rows: Vec<(String, J)> = events.iter().map(agenda_row).collect();
        // Google injoignable un instant (réponse vide) : le site garde les rendez-vous déjà envoyés.
        let keep = rows.is_empty() && live("agenda").map(|r| !r.is_empty()).unwrap_or(false);
        let rows = if keep { live("agenda").unwrap_or_default() } else { rows };
        LIVE.with(|l| l.borrow_mut().insert("agenda", (js::now_ms(), rows)));
    }
    if shared.iter().any(|s| s == "projets") && !live_fresh("projets") {
        let rows = o.projects.ordered("tous").await.iter().map(project_row).collect();
        LIVE.with(|l| l.borrow_mut().insert("projets", (js::now_ms(), rows)));
    }
}

/// Rendez-vous (Google Agenda ou agenda local) sous la forme du site : début ISO, journée entière ou non.
fn agenda_row(e: &J) -> (String, J) {
    let (start, end, all_day) = if js::truthy(&e["debut"]) {
        let d = js::gs(e, "debut");
        (d.clone(), js::gs(e, "fin"), !d.contains('T'))
    } else {
        let day = js::gs(e, "date");
        let hour = js::gs(e, "heure");
        let start = if hour.is_empty() { day.clone() } else { format!("{day}T{hour}:00") };
        let minutes = js::gn_or(e, "duree_minutes", 60.0);
        let end = js::parse_date_ms(&start).map(|ms| js::local_of(ms + (minutes * 60000.0) as i64).format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_default();
        (start, end, hour.is_empty())
    };
    let at = js::parse_date_ms(&start).unwrap_or(0);
    let when = if at <= 0 {
        start.clone()
    } else if all_day {
        js::local_of(at).format("%d/%m/%Y · journée").to_string()
    } else {
        date_fr(at)
    };
    let place = js::gs(e, "lieu");
    let id = Some(js::gs(e, "id")).filter(|i| !i.is_empty()).unwrap_or_else(|| hash(&json!([e["titre"], start]))[..16].to_string());
    let sub = [when, place.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    (id, json!({
        "_t": e["titre"], "_s": sub, "debut": start, "fin": end, "journee": all_day, "at": at, "lieu": place,
        "description": js::slice_chars(&js::gs(e, "description"), 2000), "source": e["source"]
    }))
}

/// Projet : nom, description, technologies, git ; jamais le chemin sur le disque.
fn project_row(p: &J) -> (String, J) {
    let name = js::gs(p, "name");
    let git = &p["git"];
    let mut details = vec![];
    let tech = js::arr(&p["tech"]).iter().map(js::s).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", ");
    if !tech.is_empty() {
        details.push(json!(["Technologies", tech]));
    }
    if git.is_object() {
        details.push(json!(["Branche", git["branch"]]));
        details.push(json!(["Dernier commit", js::gs(git, "lastCommit")]));
        details.push(json!(["Fichiers modifiés", git["changes"]]));
    }
    let subs = js::arr(&p["subprojects"]).iter().map(js::s).collect::<Vec<_>>();
    if !subs.is_empty() {
        details.push(json!(["Sous-projets", subs.join(", ")]));
    }
    let git_view = if git.is_object() {
        json!({ "branch": git["branch"], "lastCommit": git["lastCommit"], "lastCommitAt": git["lastCommitAt"], "changes": git["changes"] })
    } else {
        Value::Null
    };
    (name.clone(), json!({
        "_t": name, "_s": js::slice_chars(&js::gs(p, "description"), 160), "description": js::slice_chars(&js::gs(p, "description"), 1500),
        "tech": p["tech"], "git": git_view, "lastActivity": p["lastActivity"].as_i64().unwrap_or(0), "subprojects": subs,
        "favorite": js::gb(p, "favorite"), "hidden": js::gb(p, "hidden"), "_d": details
    }))
}

/// Agent autonome : ce qu'il surveille, quand, sa dernière exécution.
fn agent_row(a: &J) -> (String, J) {
    let d = &a["declencheurs"];
    let mut triggers: Vec<String> = vec![];
    if js::gb(&d["horaire"], "actif") {
        let h = &d["horaire"];
        triggers.push(format!("horaire : {:02}h{:02}", h["heure"].as_i64().unwrap_or(0), h["minutes"].as_i64().unwrap_or(0)));
    }
    if js::gb(&d["materiel"], "actif") {
        triggers.push(format!("PC : CPU {} %, RAM {} %", d["materiel"]["cpuPct"], d["materiel"]["ramPct"]));
    }
    if js::gb(&d["mails"], "actif") {
        triggers.push(format!("mails ({})", js::gs(&d["mails"], "mode")));
    }
    if js::gb(&d["demande"], "actif") {
        triggers.push("à la demande".into());
    }
    let state = if js::gb(a, "enCours") {
        "en_cours"
    } else if js::gb(a, "actif") {
        "actif"
    } else {
        "pause"
    };
    let history: Vec<J> = js::arr(&a["historique"])
        .iter()
        .take(5)
        .map(|h| json!({ "at": h["at"], "statut": h["statut"], "declencheur": h["declencheur"], "resultat": js::slice_chars(&js::gs(h, "resultat"), 1500) }))
        .collect();
    (js::gs(a, "id"), json!({
        "_t": format!("{} {}", js::gs(a, "icone"), js::gs(a, "nom")).trim(), "_s": js::slice_chars(&js::gs(a, "description"), 160),
        "description": js::gs(a, "description"), "etat": state, "declencheurs": triggers, "modele": a["modeleLibelle"], "sortie": a["sortie"]["mode"],
        "derniereExecution": a["derniereExecution"], "dernierStatut": a["dernierStatut"], "dernierResultat": js::slice_chars(&js::gs(a, "dernierResultat"), 4000),
        "nbExecutions": a["nbExecutions"], "historique": history, "consigne": js::slice_chars(&js::gs(a, "consigneIa"), 1200)
    }))
}

/// Dossier Data : la fiche complète (résumé, caractéristiques, sections, chronologie, sources, vidéos, vignette).
fn dossier_row(o: &Orchestrator, summary: &J) -> Option<(String, J)> {
    let slug = js::gs(summary, "slug");
    let d = o.savoir.get(&slug)?;
    let fiche: Vec<J> = js::arr(&d["fiche"]).iter().map(|f| json!([f["cle"], format!("{} {}", js::gs(f, "valeur"), js::gs(f, "unite")).trim()])).collect();
    let sections: Vec<J> = js::arr(&d["sections"]).iter().map(|x| json!({ "titre": x["titre"], "texte": js::slice_chars(&js::gs(x, "texte"), 3000) })).collect();
    let links: Vec<J> = js::arr(&d["sources"]).iter().filter_map(|x| https(&x["url"]).map(|u| json!([js::slice_chars(&js::gs(x, "titre"), 80), u]))).take(15).collect();
    let videos: Vec<J> = js::arr(&d["videos"]).iter().filter_map(|v| https(&v["url"]).map(|u| json!({ "titre": v["titre"], "url": u, "duree": v["duree"] }))).take(10).collect();
    let profil = &summary["profil"];
    let sub = [js::gs(profil, "nom"), js::gs(summary, "categorie")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    let mut row = json!({
        "_t": summary["sujet"], "_s": sub, "icone": profil["icone"], "profil": profil["nom"], "etat": summary["etat"], "maj": summary["maj"], "cree": summary["cree"],
        "resume": js::slice_chars(&js::gs(&d, "resume"), MAX_BODY), "fiche": fiche, "sections": sections,
        "chronologie": js::arr(&d["chronologie"]).into_iter().take(40).collect::<Vec<_>>(), "videos": videos, "_liens": links, "stats": summary["stats"]
    });
    if let Some(file) = summary["vignette"].as_str().and_then(|f| o.savoir.image_path(&slug, f)) {
        if let Some(name) = image_name(&file) {
            row["_img"] = json!(name);
        }
    }
    Some((slug, row))
}

/// Nom publié d'une image locale : empreinte du contenu, donc stable et sans rien révéler du disque.
pub fn image_name(file: &Path) -> Option<String> {
    let meta = std::fs::metadata(file).ok()?;
    let ext = file.extension()?.to_string_lossy().to_lowercase();
    if !["jpg", "png", "gif", "webp"].contains(&ext.as_str()) || meta.len() == 0 || meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    if let Some(name) = IMAGES.with(|c| c.borrow().0.get(file).filter(|(m, l, _)| *m == mtime && *l == meta.len()).map(|(_, _, n)| n.clone())) {
        return Some(name);
    }
    let bytes = std::fs::read(file).ok()?;
    use sha2::{Digest, Sha256};
    let name = format!("{}.{ext}", hex::encode(Sha256::digest(&bytes)));
    IMAGES.with(|c| {
        let mut c = c.borrow_mut();
        c.0.insert(file.to_path_buf(), (mtime, meta.len(), name.clone()));
        c.1.insert(name.clone(), file.to_path_buf());
    });
    Some(name)
}

/// Fichier local d'une image publiée (pour l'envoyer au site).
pub fn image_file(name: &str) -> Option<PathBuf> {
    IMAGES.with(|c| c.borrow().1.get(name).cloned())
}

fn https(v: &J) -> Option<String> {
    let u = js::s(v);
    (u.starts_with("https://") && u.len() < 2000 && !u.contains(char::is_whitespace)).then_some(u)
}

/// Image d'une fiche : copie locale (`_img`, envoyée au site) ou adresse web (`_imgUrl`).
fn collection_image(o: &Orchestrator, list: &J, item: &J, d: &mut serde_json::Map<String, Value>) {
    let own = js::gs(item, "image");
    if let Some(rest) = own.strip_prefix("local:") {
        let ext = rest.split('#').next().unwrap_or("");
        let file = o.collections.images_dir().join(js::gs(list, "id")).join(format!("{}.{ext}", js::gs(item, "id")));
        if let Some(name) = image_name(&file) {
            d.insert("_img".into(), json!(name));
            return;
        }
    }
    let field = list["image"].as_str().map(|k| schema::value(item, k).clone()).unwrap_or(Value::Null);
    if let Some(url) = https(&json!(own)).or_else(|| https(&field)) {
        d.insert("_imgUrl".into(), json!(url));
    }
}

pub struct Source {
    pub id: String,
    pub label: String,
    pub group: &'static str,
    pub kind: &'static str,
    /// Modifications permises depuis le site (add, update, status, check, delete).
    pub ops: &'static [&'static str],
    pub count: usize,
    pub sensitive: bool,
    pub meta: J,
}

impl Source {
    pub fn descriptor(&self, position: usize) -> J {
        json!({ "id": self.id, "label": self.label, "group": self.group, "kind": self.kind, "ops": self.ops, "meta": self.meta, "position": position })
    }

    pub fn public(&self, shared: bool) -> J {
        json!({ "id": self.id, "label": self.label, "group": self.group, "kind": self.kind, "count": self.count,
                "sensitive": self.sensitive, "writable": !self.ops.is_empty(), "ops": self.ops, "shared": shared })
    }
}

/// Toutes les sources partageables, dans l'ordre d'affichage de l'administration.
pub fn catalog(o: &Orchestrator) -> Vec<Source> {
    let mut out = vec![];
    for list in o.collections.all() {
        let voc = schema::vocab(&list);
        out.push(Source {
            id: format!("col:{}", js::gs(&list, "id")),
            label: format!("{} {}", js::gs(&list, "icone"), js::gs(&list, "nom")).trim().to_string(),
            group: "Listes à fiches",
            kind: "collection",
            ops: &["add", "update", "status", "delete"],
            count: js::arr(&list["elements"]).len(),
            sensitive: false,
            meta: json!({
                "champs": js::arr(&list["champs"]).iter().map(|f| json!({ "cle": f["cle"], "libelle": f["libelle"], "type": f["type"], "choix": f["choix"] })).collect::<Vec<_>>(),
                "titre": list["titre"], "etat": list["etat"], "icone": list["icone"], "image": list["image"],
                // Scan d'un code-barres sur le site : seulement si Jarvis sait compléter la fiche ensuite.
                "scan": super::super::collections::Collections::has_provider(&list),
                "libelles": { "possede": voc["fait"], "souhaite": voc["voulu"] }
            }),
        });
    }
    for list in o.smart_lists.lists.borrow().iter() {
        out.push(Source {
            id: format!("liste:{}", js::gs(list, "id")),
            label: js::gs(list, "title"),
            group: "Listes",
            kind: "liste",
            ops: &["add", "update", "check", "delete"],
            count: js::arr(&list["items"]).len(),
            sensitive: false,
            meta: json!({ "type": list["type"] }),
        });
    }
    let fixed: [(&str, &str, &str, &str, &'static [&'static str], usize, bool); 7] = [
        ("notes", "Notes", "Notes et rappels", "notes", &["add"], o.notes.list().len(), false),
        ("rappels", "Rappels", "Notes et rappels", "rappels", &["add", "delete"], o.reminders.upcoming().len(), false),
        ("programmations", "Programmations", "Notes et rappels", "lecture", &[], o.schedules.upcoming().len(), false),
        ("colis", "Colis suivis", "Suivi", "lecture", &[], o.parcels.list().len(), false),
        ("produits", "Produits suivis", "Suivi", "lecture", &[], o.shopping.items.borrow().len(), false),
        ("reunions", "Réunions (titre, résumé)", "Suivi", "lecture", &[], o.meetings.list().len(), true),
        ("memoire", "Mémoire de Jarvis (faits retenus)", "Mémoire", "lecture", &[], o.memory.len(), true),
    ];
    for (id, label, group, kind, ops, count, sensitive) in fixed {
        out.push(Source { id: id.into(), label: label.into(), group, kind, ops, count, sensitive, meta: Value::Null });
    }
    // Rubriques du menu du site (lecture seule) : agenda, agents, projets, dossiers Data.
    let top_projects = o.projects.list().iter().filter(|p| p["parent"].is_null()).count();
    let sections: [(&str, &str, &str, &str, usize, bool); 4] = [
        ("agenda", "Agenda (90 jours)", "Agenda", "agenda", live("agenda").map(|r| r.len()).unwrap_or(0), true),
        ("agents", "Agents autonomes", "Agents", "agents", o.agents.count(), false),
        ("projets", "Projets", "Projets", "projets", live("projets").map(|r| r.len()).unwrap_or(top_projects), false),
        ("data", "Dossiers Data", "Data", "dossiers", o.savoir.list().len(), false),
    ];
    for (id, label, group, kind, count, sensitive) in sections {
        out.push(Source { id: id.into(), label: label.into(), group, kind, ops: &[], count, sensitive, meta: Value::Null });
    }
    out
}

fn date_fr(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    js::local_of(ms).format("%d/%m/%Y à %Hh%M").to_string()
}

/// Enregistrements actuels d'une source : (identifiant, données). None : source inconnue.
pub fn records(o: &Orchestrator, id: &str) -> Option<Vec<(String, J)>> {
    if let Some(list_id) = id.strip_prefix("col:") {
        let list = o.collections.get(list_id)?;
        return Some(
            js::arr(&list["elements"])
                .iter()
                .map(|item| {
                    let mut d = item["valeurs"].as_object().cloned().unwrap_or_default();
                    d.insert("_t".into(), json!(schema::title_of(&list, item)));
                    d.insert("_s".into(), json!(schema::subtitle_of(&list, item)));
                    d.insert("_statut".into(), json!(schema::statut_of(&list, item).code()));
                    collection_image(o, &list, item, &mut d);
                    // Boutiques des derniers prix relevés : liens directs vers la page du produit.
                    let mut links: Vec<J> = vec![];
                    for p in js::arr(&item["historiquePrix"]).iter().rev() {
                        let (shop, url) = (js::gs(p, "boutique"), js::gs(p, "url"));
                        if url.starts_with("https://") && !links.iter().any(|l| l[1] == json!(url)) && links.len() < 4 {
                            let price = js::num(&p["prix"]).map(|n| format!(" — {n:.2} €")).unwrap_or_default();
                            links.push(json!([format!("{}{price}", if shop.is_empty() { "Boutique".to_string() } else { shop }), url]));
                        }
                    }
                    if !links.is_empty() {
                        d.insert("_liens".into(), json!(links));
                    }
                    if let Some(code) = item["codeBarre"].as_str() {
                        d.insert("_barcode".into(), json!(code)); // le site signale un article déjà scanné
                    }
                    (js::gs(item, "id"), Value::Object(d))
                })
                .collect(),
        );
    }
    if let Some(list_id) = id.strip_prefix("liste:") {
        let list = o.smart_lists.get(list_id)?;
        return Some(
            js::arr(&list["items"])
                .iter()
                .map(|i| (js::s(&i["id"]), json!({ "_t": i["text"], "text": i["text"], "checked": js::gb(i, "checked"), "category": i["category"] })))
                .collect(),
        );
    }
    let rows = match id {
        "notes" => o
            .notes
            .list()
            .into_iter()
            .map(|n| {
                let rid = n.file.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                (rid, json!({ "_t": n.title, "_s": n.date_label, "title": n.title, "body": js::slice_chars(&n.body, MAX_BODY), "kind": n.kind, "words": n.words }))
            })
            .collect(),
        "rappels" => o
            .reminders
            .upcoming()
            .into_iter()
            .map(|r| {
                let at = r["at"].as_i64().unwrap_or(0);
                let every = js::gs(&r["repeat"], "every");
                let repeat = match every.as_str() {
                    "day" => "chaque jour",
                    "week" => "chaque semaine",
                    _ => "",
                };
                let sub = [date_fr(at), repeat.to_string()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                (js::s(&r["id"]), json!({ "_t": r["what"], "_s": sub, "what": r["what"], "at": at, "repeat": r["repeat"] }))
            })
            .collect(),
        "programmations" => o
            .schedules
            .upcoming()
            .into_iter()
            .map(|i| {
                let d = o.schedules.describe(&i);
                (js::s(&i["id"]), json!({ "_t": d["actionLabel"], "_s": d["when"] }))
            })
            .collect(),
        "colis" => o
            .parcels
            .list()
            .into_iter()
            .map(|p| {
                let carrier = js::gs(&p, "transporteur");
                let status = js::gs(&p, "statut");
                (js::gs(&p, "id"), json!({
                    "_t": if carrier.is_empty() { js::gs(&p, "numero") } else { format!("{} · {carrier}", js::gs(&p, "numero")) },
                    "_s": if js::gb(&p, "livre") { "Livré".to_string() } else if status.is_empty() { "En cours".to_string() } else { status.clone() },
                    "_d": [["Numéro", p["numero"]], ["Transporteur", carrier], ["Statut", status], ["Livré", if js::gb(&p, "livre") { "oui" } else { "non" }], ["Ajouté le", p["ajouteLe"]]]
                }))
            })
            .collect(),
        "produits" => o
            .shopping
            .items
            .borrow()
            .iter()
            .map(|p| {
                let history = js::arr(&p["history"]);
                let last = history.last().cloned().unwrap_or(Value::Null);
                let price = js::num(&last["price"]).map(|n| format!("{n:.2} €")).unwrap_or_default();
                let mut details = vec![json!(["Marque", p["brand"]]), json!(["Référence", p["reference"]]), json!(["Description", js::slice_chars(&js::gs(p, "description"), 600)])];
                for h in history.iter().rev().take(6) {
                    details.push(json!([js::gs(h, "at").chars().take(10).collect::<String>(), format!("{} {}", js::num(&h["price"]).map(|n| format!("{n:.2} €")).unwrap_or_default(), js::gs(h, "store")).trim()]));
                }
                let sub = [js::gs(p, "brand"), price].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                let links: Vec<J> = js::arr(&p["offers"]).iter().filter_map(|of| https(&of["url"]).map(|u| json!([if js::gs(of, "store").is_empty() { "Boutique".to_string() } else { js::gs(of, "store") }, u]))).take(8).collect();
                let mut d = json!({ "_t": p["name"], "_s": sub, "_d": details, "_liens": links });
                if let Some(u) = https(&p["image"]) {
                    d["_imgUrl"] = json!(u);
                }
                (js::gs(p, "id"), d)
            })
            .collect(),
        "reunions" => o
            .meetings
            .list()
            .into_iter()
            .map(|m| {
                let file = js::gs(&m, "file");
                let summary = js::read_json(std::path::Path::new(&file)).map(|j| summary_text(&j["summary"])).unwrap_or_default();
                let rid = std::path::Path::new(&file).file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                let names = js::arr(&m["participants"]).iter().map(|p| if p.is_string() { js::s(p) } else { js::gs(p, "name") }).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                (rid, json!({
                    "_t": m["title"], "_s": date_fr(js::gn_or(&m, "created", 0.0) as i64),
                    "_d": [["Participants", names], ["Répliques", m["lines"]], ["Résumé", js::slice_chars(&summary, MAX_BODY)]]
                }))
            })
            .collect(),
        "memoire" => o.memory.list().into_iter().map(|e| (js::gs(&e, "id"), json!({ "_t": e["text"], "_s": e["category"] }))).collect(),
        // Pas encore relu : rien n'est retiré du site en attendant (None = source inconnue pour l'appelant).
        "agenda" => live("agenda")?,
        "projets" => live("projets")?,
        "agents" => o.agents.list().iter().map(agent_row).collect(),
        "data" => o.savoir.list().iter().filter_map(|d| dossier_row(o, d)).collect(),
        _ => return None,
    };
    Some(rows)
}

/// Résumé de réunion : texte, ou objet { texte | resume | points… } selon la version qui l'a produit.
fn summary_text(v: &J) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(m) => ["text", "texte", "resume", "summary"].iter().find_map(|k| m.get(*k).and_then(|x| x.as_str()).map(String::from)).unwrap_or_else(|| js::pretty(v)),
        Value::Null => String::new(),
        _ => js::s(v),
    }
}

/// Empreinte d'un enregistrement : ce qui a déjà été envoyé n'est pas renvoyé.
pub fn hash(v: &J) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(serde_json::to_string(v).unwrap_or_default().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendez_vous_local_et_google_au_meme_format() {
        let (_, local) = agenda_row(&json!({ "id": "evt_1", "titre": "Dentiste", "date": "2026-11-03", "heure": "14:30", "duree_minutes": 30, "lieu": "Lyon" }));
        assert_eq!(local["debut"], json!("2026-11-03T14:30:00"));
        assert_eq!(local["fin"], json!("2026-11-03T15:00:00"));
        assert_eq!(local["journee"], json!(false));
        assert!(js::gs(&local, "_s").ends_with("Lyon"));
        let (id, google) = agenda_row(&json!({ "titre": "Anniversaire", "debut": "2026-11-05", "fin": "2026-11-06", "source": "Google Calendar" }));
        assert_eq!(google["journee"], json!(true));
        assert_eq!(id.len(), 16, "sans identifiant : empreinte stable");
    }

    #[test]
    fn un_projet_ne_donne_jamais_son_chemin() {
        let (_, p) = project_row(&json!({ "name": "jarvis-v4", "path": "E:/www/jarvis-v4", "description": "Assistant", "tech": ["Rust"], "git": { "branch": "main", "changes": 2 } }));
        assert!(!serde_json::to_string(&p).unwrap().contains("www"));
        assert_eq!(p["git"]["branch"], json!("main"));
    }
}
