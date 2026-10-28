// Service des listes à fiches : le SEUL endroit qui lit, valide, enregistre, indexe, journalise et annule.
// La voix (voice.rs), la card du HUD (méthode « collections ») et la page d'aide (API HTTP protégée)
// appellent toutes `dispatch` ou les fonctions ci-dessous : une liste se comporte pareil quelle que soit
// la façon dont elle est modifiée. Une liste = data/collections/<id>.json (schéma + éléments).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Map, Value};

use super::super::automations::base36;
use super::super::{OrchRef, Up};
use super::cards;
use super::index::{Hit, Index, Passage};
use super::schema::{self, fold, Statut};
use super::voice::Conversation;
use crate::js::{self, J};

/// Opérations que la page d'aide peut appeler par l'API HTTP (jeton exigé) : pas de pilotage du HUD
/// (ouvrir, proposer…), pas d'action sur le système (action).
pub const HTTP_OPS: &[&str] = &[
    "lister", "lire", "modeles", "creer_liste", "modifier_schema", "supprimer_liste", "ajouter_element", "ajouter_texte",
    "modifier_element", "supprimer_element", "changer_statut", "exporter_csv", "reindexer", "rechercher", "stats", "enrichir", "enrichir_liste", "supprimer_elements", "modifier_elements", "statistiques", "importer_csv",
];

const BULK_PROMPT: &str = "Tu mets en forme un texte libre pour l'insérer dans une collection structurée. \
Tu n'as aucun outil. Analyse et prends en compte TOUT le texte fourni (tableaux markdown, listes, détails, setlists, notes...). \
Réorganise les données pour chaque élément en remplissant au mieux les champs disponibles de la liste : \
- titre obligatoire pour chaque élément \
- pour les dates : formate en YYYY-MM-DD si possible, ou conserve l'année \
- si des sections détaillées (comme les setlists, morceaux, affluence, détails) correspondent à un élément, inclus-les synthétiquement dans le champ notes / texte long correspondant \
- ne perds aucune information pertinente du texte \
- pour chaque élément, renseigne un objet avec les clés de champs données \
Réponds uniquement par un tableau JSON valide d'objets, sans texte ni balise markdown autour : [{\"cle_champ\": valeur, ...}, ...].";

/// Au-delà, aucun retour en arrière n'est gardé en mémoire pour une liste (taille du fichier).
const UNDO_MAX_BYTES: usize = 2_000_000;

pub struct Collections {
    pub(super) o: OrchRef,
    pub dir: PathBuf,
    pub(super) log: Rc<dyn Fn(String)>,
    pub(super) lists: RefCell<Vec<J>>,
    pub index: Index,
    pub(super) conv: RefCell<Option<Conversation>>,
    /// Tâches IA de fond en cours : (consigne envoyée, sujet).
    pub(super) jobs: RefCell<Vec<(String, String)>>,
    /// Tâches IA de fond d'enrichissement ou de prix : (consigne envoyée, tâche).
    pub(super) enrich_jobs: RefCell<Vec<(String, super::enrich::EnrichJob)>>,
    /// Fiche ouverte dans le HUD, par liste (pour la rafraîchir après un changement).
    open_items: RefCell<HashMap<String, String>>,
    reindex: js::Timer,
    /// Passage régulier qui complète par l'IA les fiches incomplètes.
    pub(super) sweep: js::Timer,
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Mots d'une expression sans pluriel : « disques vinyles » ~ « disque vinyle ».
pub fn loose(s: &str) -> String {
    fold(s).split(' ').map(|w| if w.len() > 3 { w.trim_end_matches(['s', 'x']) } else { w }).collect::<Vec<_>>().join(" ")
}

/// Code-barres EAN / UPC scanné (8 à 14 chiffres), espaces et tirets retirés ; None s'il n'en est pas un.
pub fn clean_barcode(raw: &str) -> Option<String> {
    let code: String = raw.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    ((8..=14).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit())).then_some(code)
}

fn new_element_id() -> String {
    format!("e{}{}", base36(js::now_ms() as u64), js::rand_below(1000))
}

/// Lit un CSV : séparateur « ; », tabulation ou « , » (celui qui domine sur la 1re ligne), guillemets, retours à la ligne entre guillemets.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let first = text.lines().next().unwrap_or("");
    let commas = first.matches(',').count();
    let sep = if first.matches(';').count() > commas { ';' } else if first.matches('\t').count() > commas { '\t' } else { ',' };
    let (mut rows, mut row, mut cell) = (Vec::<Vec<String>>::new(), Vec::<String>::new(), String::new());
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

impl Collections {
    pub fn new(o: OrchRef, dir: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log(format!("Collections mkdir error: {e}"));
        }
        let me = Self {
            index: Index::new(dir.join("vectors.json")),
            o,
            dir,
            log,
            lists: RefCell::new(vec![]),
            conv: RefCell::new(None),
            jobs: RefCell::new(vec![]),
            enrich_jobs: RefCell::new(vec![]),
            open_items: RefCell::new(HashMap::new()),
            reindex: js::Timer::new(),
            sweep: js::Timer::new(),
        };
        me.load_all();
        me.prune_images();
        me
    }

    pub fn load_all(&self) {
        let mut out: Vec<J> = vec![];
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(".json") || name == "vectors.json" {
                    continue;
                }
                let Some(mut d) = js::read_json(&e.path()) else { continue };
                if !valid_id(&js::gs(&d, "id")) || !d["champs"].is_array() {
                    continue;
                }
                if !d["elements"].is_array() {
                    d["elements"] = json!([]);
                }
                out.push(d);
            }
        }
        out.sort_by_key(|l| js::gs(l, "creeLe"));
        *self.lists.borrow_mut() = out;
    }

    // --- Images mises en cache -------------------------------------------------------------------

    pub fn images_dir(&self) -> PathBuf {
        self.dir.join("images")
    }

    /// Image d'une fiche : « <liste>/<élément>.<ext> » → octets et type. Les identifiants sont revalidés ici.
    pub fn image_file(&self, rest: &str) -> Option<(Vec<u8>, &'static str)> {
        let (list, file) = rest.split_once('/')?;
        let (elt, ext) = file.rsplit_once('.')?;
        if !valid_id(list) || !valid_id(elt) || file.contains('/') {
            return None;
        }
        let ctype = match ext {
            "jpg" => "image/jpeg",
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => return None,
        };
        self.get(list)?;
        std::fs::read(self.images_dir().join(list).join(format!("{elt}.{ext}"))).ok().map(|b| (b, ctype))
    }

    // --- Lecture ---------------------------------------------------------------------------------

    pub fn all(&self) -> Vec<J> {
        self.lists.borrow().clone()
    }

    pub fn count(&self) -> usize {
        self.lists.borrow().len()
    }

    pub fn item_count(&self) -> usize {
        self.lists.borrow().iter().map(|l| js::arr(&l["elements"]).len()).sum()
    }

    pub fn get(&self, id: &str) -> Option<J> {
        self.lists.borrow().iter().find(|l| js::gs(l, "id") == id).cloned()
    }

    /// La liste que l'utilisateur désigne par un nom ou un mot courant (« vinyle », « mes disques », « lego »).
    pub fn find_by_phrase(&self, spoken: &str) -> Option<J> {
        let mut q = fold(spoken);
        loop {
            let t = crate::re!(r"^(?:la|le|les|l|ma|mon|mes|ta|ton|tes|de|d|des|du|pour|sur|liste|listes|collection)\s+").replace(&q, "").to_string();
            if t == q {
                break;
            }
            q = t;
        }
        let q = crate::re!(r"\s+(?:stp|svp|s il te plait)$").replace(&q, "").to_string();
        if q.is_empty() {
            return None;
        }
        let ql = loose(&q);
        let padded = format!(" {ql} ");
        let mut best: Option<(usize, J)> = None;
        for list in self.lists.borrow().iter() {
            let names: Vec<String> = js::str_list(&list["alias"]).into_iter().chain([js::gs(list, "nom")]).map(|a| loose(&a)).filter(|a| !a.is_empty()).collect();
            let mut score = 0usize;
            for n in &names {
                let s = if *n == ql {
                    1000 + n.len()
                } else if padded.contains(&format!(" {n} ")) {
                    100 + n.len()
                } else if n.len() >= 3 && format!(" {n} ").contains(&padded) {
                    10 + ql.len()
                } else {
                    0
                };
                score = score.max(s);
            }
            if score > 0 && best.as_ref().map(|(b, _)| score > *b).unwrap_or(true) {
                best = Some((score, list.clone()));
            }
        }
        best.map(|(_, l)| l)
    }

    /// Le plus long nom de liste qui commence le texte : (liste, reste). « vinyl le disque X » → (Vinyles, « le disque X »).
    pub fn find_by_prefix(&self, text: &str) -> Option<(J, String)> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut best: Option<(usize, J, usize)> = None; // (longueur, liste, nombre de mots)
        for list in self.lists.borrow().iter() {
            for a in js::str_list(&list["alias"]).into_iter().chain([js::gs(list, "nom")]) {
                let n = loose(&a);
                if n.is_empty() {
                    continue;
                }
                let count = n.split(' ').count();
                if count > words.len() {
                    continue;
                }
                let head = loose(&words[..count].join(" "));
                if head == n && best.as_ref().map(|(l, _, _)| n.len() > *l).unwrap_or(true) {
                    best = Some((n.len(), list.clone(), count));
                }
            }
        }
        best.map(|(_, l, c)| (l, words[c..].join(" ")))
    }

    // --- Enregistrement ------------------------------------------------------------------------

    fn path_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub(super) fn save_list(&self, list: &mut J) -> Result<(), String> {
        let id = js::gs(list, "id");
        if !valid_id(&id) {
            return Err("Identifiant de liste invalide.".into());
        }
        list["majLe"] = json!(js::iso_now());
        js::write_json_atomic(&self.path_of(&id), list).map_err(|e| {
            (self.log)(format!("Collections save error: {e}"));
            "Je n'ai pas pu enregistrer la liste.".to_string()
        })?;
        {
            let mut lists = self.lists.borrow_mut();
            match lists.iter_mut().find(|l| js::gs(l, "id") == id) {
                Some(l) => *l = list.clone(),
                None => lists.push(list.clone()),
            }
        }
        self.sync_memory(list);
        self.reindex_soon();
        Ok(())
    }

    /// La mémoire générale sait que la liste existe (pas ce qu'elle contient : c'est le rôle de l'index).
    fn sync_memory(&self, list: &J) {
        if let Some(o) = self.o.upgrade() {
            o.memory.upsert_pinned(&format!("liste_{}", js::gs(list, "id")), &schema::summary(list), "listes");
        }
    }

    fn reindex_soon(&self) {
        let o = self.o.clone();
        self.reindex.set(1500, move || {
            if let Some(orch) = o.upgrade() {
                js::spawn(async move {
                    let passages = orch.collections.passages(None);
                    let _ = orch.collections.index.ensure(&passages).await;
                });
            }
        });
    }

    pub fn passages(&self, only: Option<&str>) -> Vec<Passage> {
        let mut out = vec![];
        for list in self.lists.borrow().iter() {
            if only.map(|o| o != js::gs(list, "id")).unwrap_or(false) {
                continue;
            }
            for item in js::arr(&list["elements"]) {
                out.push(Passage {
                    list: js::gs(list, "id"),
                    element: js::gs(&item, "id"),
                    text: schema::passage(list, &item),
                    ids: schema::identifiers_of(list, &item).iter().map(|s| fold(s)).collect(),
                });
            }
        }
        out
    }

    pub(super) fn record(&self, typ: &str, data: J) {
        if let Some(o) = self.o.upgrade() {
            o.record(typ, data);
        }
    }

    /// Prévient les fenêtres et rafraîchit les cards ouvertes.
    pub(super) fn changed(&self, list_id: &str, element: Option<&str>, op: &str) {
        let Some(o) = self.o.upgrade() else { return };
        o.emit(json!({ "type": "collections_changed", "liste": list_id, "element": element, "op": op }));
        self.refresh(list_id);
    }

    fn refresh(&self, list_id: &str) {
        let Some(o) = self.o.upgrade() else { return };
        let keys = o.cards.keys();
        if let Some(list) = self.get(list_id) {
            if keys.contains(&format!("tool:{}", cards::list_title(&list))) {
                o.show_widget("tool", cards::list_card(&list));
            }
            let open = self.open_items.borrow().get(list_id).cloned();
            if let Some(eid) = open {
                if keys.contains(&format!("tool:{}", cards::item_title(&list))) {
                    match js::arr(&list["elements"]).iter().any(|e| js::gs(e, "id") == eid) {
                        true => o.show_widget("tool", cards::item_card(&list, &eid)),
                        false => o.emit(json!({ "type": "widget_close", "key": format!("tool:{}", cards::item_title(&list)) })),
                    }
                }
            }
        }
        if keys.contains(&format!("tool:{}", cards::OVERVIEW_TITLE)) {
            o.show_widget("tool", cards::overview_card(&o));
        }
    }

    fn push_undo(&self, label: &str, before: J) {
        let Some(o) = self.o.upgrade() else { return };
        if before.to_string().len() > UNDO_MAX_BYTES {
            return;
        }
        let me = self.o.clone();
        let text = label.to_string();
        o.undo.add(
            label,
            Box::new(move || {
                Box::pin(async move {
                    let Some(orch) = me.upgrade() else { return String::new() };
                    match orch.collections.restore(&before) {
                        Ok(_) => format!("C'est annulé : {text}."),
                        Err(e) => e,
                    }
                })
            }),
        );
    }

    /// Remet une liste dans l'état d'un instantané (annulation).
    fn restore(&self, snapshot: &J) -> Result<(), String> {
        let mut list = snapshot.clone();
        self.save_list(&mut list)?;
        self.changed(&js::gs(&list, "id"), None, "annulation");
        Ok(())
    }

    // --- Listes --------------------------------------------------------------------------------

    pub fn create_list(&self, input: &J, source: &str) -> Result<J, String> {
        self.create_list_from(input, source, None)
    }

    /// Création d'une liste ; `origine` dit qui l'a demandée (invité du site Nomade : « liste invité » dans la card).
    pub fn create_list_from(&self, input: &J, source: &str, origine: Option<J>) -> Result<J, String> {
        if self.count() >= schema::MAX_LISTS {
            return Err("Tu as atteint le nombre maximal de listes.".into());
        }
        let mut list = schema::normalize_schema(input)?;
        let base = {
            let s = schema::slug(&js::gs(&list, "nom"));
            let s = js::slice_chars(&s, 24);
            if s.is_empty() { "liste".to_string() } else { s }
        };
        let mut id = format!("{base}_{}", base36(js::now_ms() as u64));
        while self.get(&id).is_some() {
            id = format!("{base}_{}{}", base36(js::now_ms() as u64), js::rand_below(100));
        }
        let now = js::iso_now();
        list["id"] = json!(id);
        list["creeLe"] = json!(now);
        list["elements"] = json!([]);
        if let Some(o) = origine {
            list["origine"] = o;
        }
        if self.find_by_phrase(&js::gs(&list, "nom")).map(|l| fold(&js::gs(&l, "nom")) == fold(&js::gs(&list, "nom"))).unwrap_or(false) {
            return Err(format!("Une liste « {} » existe déjà.", js::gs(&list, "nom")));
        }
        self.save_list(&mut list)?;
        self.record("liste_creee", json!({ "liste": id, "champs": js::arr(&list["champs"]).len(), "modele": list["modele"], "source": source }));
        self.changed(&id, None, "liste_creee");
        Ok(list)
    }

    // --- Modèles personnalisés (créés à la demande, ex. depuis le site Nomade) ---------------------------

    fn models_file(&self) -> PathBuf {
        self.dir.parent().unwrap_or(&self.dir).join("collection-models.json")
    }

    pub fn custom_models(&self) -> Vec<J> {
        js::read_json(&self.models_file()).map(|v| js::arr(&v["modeles"])).unwrap_or_default()
    }

    /// Modèles livrés puis modèles personnalisés.
    pub fn all_models(&self) -> Vec<J> {
        schema::presets().iter().cloned().chain(self.custom_models()).collect()
    }

    /// Modèle personnalisé dont un mot-clé figure dans le texte.
    pub fn find_custom_model(&self, text: &str) -> Option<J> {
        let padded = format!(" {} ", fold(text));
        self.custom_models().into_iter().find(|m| js::str_list(&m["motsCles"]).iter().any(|k| !fold(k).is_empty() && padded.contains(&format!(" {} ", fold(k)))))
    }

    /// Enregistre (ou remplace) un modèle : le schéma est revalidé, 40 modèles au plus.
    pub fn save_custom_model(&self, input: &J) -> Result<J, String> {
        let mut model = schema::normalize_schema(input)?;
        let id = format!("perso_{}", js::slice_chars(&schema::slug(&js::gs(&model, "nom")), 24));
        model["id"] = json!(id);
        model["motsCles"] = json!(js::str_list(&model["alias"]));
        let mut all = self.custom_models();
        all.retain(|m| js::gs(m, "id") != id);
        if all.len() >= 40 {
            return Err("Trop de modèles personnalisés.".into());
        }
        all.push(model.clone());
        js::write_json(&self.models_file(), &json!({ "modeles": all })).map_err(|e| format!("Enregistrement du modèle impossible : {e}"))?;
        Ok(model)
    }

    /// Modifie le schéma : nom, icône, alias, champs ajoutés / retirés / renommés, choix, rôles.
    pub fn update_schema(&self, list_id: &str, patch: &J, source: &str) -> Result<J, String> {
        let before = self.get(list_id).ok_or("Liste introuvable.")?;
        let mut draft = before.clone();
        for k in ["nom", "icone", "noms", "alias", "titre", "sousTitre", "identifiant", "etat", "vignette", "image", "tri"] {
            if !patch[k].is_null() {
                draft[k] = patch[k].clone();
            }
        }
        let mut fields = js::arr(&draft["champs"]);
        if let Some(remove) = patch["retirer"].as_array() {
            let title = js::gs(&before, "titre");
            for r in remove.iter().filter_map(|r| r.as_str()) {
                if r == title {
                    return Err("Le champ qui sert de nom ne peut pas être retiré.".into());
                }
                fields.retain(|f| js::gs(f, "cle") != r);
            }
        }
        if let Some(rename) = patch["renommer"].as_object() {
            for (key, label) in rename {
                if let Some(f) = fields.iter_mut().find(|f| js::gs(f, "cle") == *key) {
                    let l = schema::clean_text(&js::s(label), 40);
                    if !l.is_empty() {
                        f["libelle"] = json!(l);
                    }
                }
            }
        }
        if let Some(req) = patch["obligatoire"].as_object() {
            for (key, on) in req {
                if let Some(f) = fields.iter_mut().find(|f| js::gs(f, "cle") == *key) {
                    if on.as_bool() == Some(true) {
                        f["obligatoire"] = json!(true);
                    } else if let Some(o) = f.as_object_mut() {
                        o.remove("obligatoire");
                    }
                }
            }
        }
        if let Some(choices) = patch["choix"].as_object() {
            for (key, list) in choices {
                if let Some(f) = fields.iter_mut().find(|f| js::gs(f, "cle") == *key && js::gs(f, "type") == "choix") {
                    f["choix"] = list.clone();
                }
            }
        }
        for add in js::arr(&patch["ajouter"]) {
            if fields.len() >= schema::MAX_FIELDS {
                return Err(format!("Une liste ne peut pas avoir plus de {} champs.", schema::MAX_FIELDS));
            }
            let Some(f) = schema::normalize_field(&add) else { continue };
            if fields.iter().any(|x| js::gs(x, "cle") == js::gs(&f, "cle")) {
                return Err(format!("Le champ « {} » existe déjà.", js::gs(&f, "libelle")));
            }
            fields.push(f);
        }
        draft["champs"] = json!(fields);
        if patch["enrichissement"].is_object() {
            draft["enrichissement"] = patch["enrichissement"].clone();
        }
        let mut next = schema::normalize_schema(&draft)?;
        // Les valeurs des champs retirés disparaissent ; celles des autres sont revalidées.
        let mut elements = js::arr(&before["elements"]);
        for e in elements.iter_mut() {
            let mut kept = Map::new();
            if let Some(vals) = e["valeurs"].as_object() {
                for (k, v) in vals {
                    if let Some(f) = schema::field(&next, k) {
                        if let Ok(nv) = schema::normalize_value(f, v) {
                            if !nv.is_null() {
                                kept.insert(k.clone(), nv);
                            }
                        }
                    }
                }
            }
            e["valeurs"] = Value::Object(kept);
        }
        for k in ["id", "creeLe", "origine"] {
            if !before[k].is_null() {
                next[k] = before[k].clone();
            }
        }
        next["elements"] = json!(elements);
        self.save_list(&mut next)?;
        self.push_undo(&format!("modification de la liste {}", js::gs(&before, "nom")), before);
        self.record("liste_modifiee", json!({ "liste": list_id, "source": source }));
        self.changed(list_id, None, "liste_modifiee");
        Ok(next)
    }

    /// Parcourt la liste et cherche l'image (et les infos) des fiches qui n'en ont pas, une recherche IA par fiche.
    /// Une liste issue d'un modèle sans fournisseur adopte celui du modèle (ex. Concerts : recherche web).
    pub fn enrich_list(&self, list_id: &str, a: &J, source: &str) -> Result<J, String> {
        let mut list = self.get(list_id).ok_or("Liste introuvable.")?;
        if self.o.upgrade().is_none() {
            return Err("L'IA n'est pas disponible pour le moment.".into());
        }
        if !Self::can_fill(&list) {
            // Les listes créées depuis une proposition ont le modèle « libre » : on retrouve le modèle livré par le nom ou les alias.
            let name = fold(&js::gs(&list, "nom"));
            let aliases: Vec<String> = js::str_list(&list["alias"]).iter().map(|a| fold(a)).collect();
            let preset = schema::preset(&js::gs(&list, "modele"))
                .into_iter()
                .chain(schema::presets().iter().cloned())
                .find(|p| Self::has_provider(p) && (fold(&js::gs(p, "nom")) == name || js::str_list(&p["alias"]).iter().any(|a| aliases.contains(&fold(a)))))
                .ok_or("Cette liste n'a pas d'enrichissement automatique.")?;
            list = self.update_schema(list_id, &json!({ "enrichissement": preset["enrichissement"] }), source)?;
        }
        let all = js::gb(a, "ecraser");
        let ids: Vec<String> = js::arr(&list["elements"])
            .iter()
            .filter(|e| all || (list["image"].is_string() && js::gs(e, "image").is_empty()) || Self::has_gaps(&list, e["valeurs"].as_object().unwrap_or(&Map::new())))
            .map(|e| js::gs(e, "id"))
            .collect();
        for id in &ids {
            self.enrich_soon(list_id, id, source, super::enrich::EnrichOpts { ecraser: all, suivant: false });
        }
        let n = ids.len();
        Ok(json!({ "ok": true, "total": n, "say": if n == 0 { "Toutes les fiches sont déjà complètes.".to_string() } else { format!("Je cherche les images de {n} fiche{} ; ça peut prendre quelques minutes.", if n > 1 { "s" } else { "" }) } }))
    }

    /// Supprime une liste. `confirm` doit être le nom de la liste (formulaires) ; la voix a déjà demandé « oui ».
    pub fn delete_list(&self, list_id: &str, confirm: Option<&str>, source: &str) -> Result<J, String> {
        let list = self.get(list_id).ok_or("Liste introuvable.")?;
        if let Some(c) = confirm {
            if fold(c) != fold(&js::gs(&list, "nom")) {
                return Err("Retape le nom de la liste pour confirmer la suppression.".into());
            }
        }
        if let Err(e) = std::fs::remove_file(self.path_of(list_id)) {
            if self.path_of(list_id).exists() {
                return Err(format!("Je n'ai pas pu supprimer la liste : {e}"));
            }
        }
        self.lists.borrow_mut().retain(|l| js::gs(l, "id") != list_id);
        if let Some(o) = self.o.upgrade() {
            o.memory.upsert_pinned(&format!("liste_{list_id}"), "", "listes");
            let mut keys = vec![format!("tool:{}", cards::list_title(&list)), format!("tool:{}", cards::item_title(&list))];
            keys.retain(|k| o.cards.has_key(k));
            for k in keys {
                o.emit(json!({ "type": "widget_close", "key": k }));
            }
        }
        self.open_items.borrow_mut().remove(list_id);
        self.reindex_soon();
        self.push_undo(&format!("suppression de la liste {}", js::gs(&list, "nom")), list.clone());
        self.record("liste_supprimee", json!({ "liste": list_id, "elements": js::arr(&list["elements"]).len(), "source": source }));
        self.changed(list_id, None, "liste_supprimee");
        Ok(json!({ "nom": list["nom"], "elements": js::arr(&list["elements"]).len() }))
    }

    // --- Éléments ------------------------------------------------------------------------------

    /// Applique des valeurs saisies (validées champ par champ) ; une valeur vide retire le champ.
    fn apply_values(list: &J, target: &mut Map<String, Value>, patch: &J) -> Result<(), String> {
        let Some(obj) = patch.as_object() else { return Ok(()) };
        for (key, v) in obj {
            let f = schema::field(list, key).ok_or_else(|| format!("Le champ « {key} » n'existe pas dans cette liste."))?;
            match schema::normalize_value(f, v)? {
                Value::Null => {
                    target.remove(key);
                }
                nv => {
                    target.insert(key.clone(), nv);
                }
            }
        }
        Ok(())
    }

    /// Élément déjà présent : même identifiant (numéro de set…), ou même titre et même sous-titre.
    pub fn find_duplicate(list: &J, values: &Map<String, Value>) -> Option<J> {
        let probe = json!({ "valeurs": Value::Object(values.clone()) });
        let ids: Vec<String> = schema::identifiers_of(list, &probe).iter().map(|s| fold(s)).collect();
        let title = fold(&schema::title_of(list, &probe));
        let sub = fold(&schema::subtitle_of(list, &probe));
        js::arr(&list["elements"]).into_iter().find(|e| {
            let eids: Vec<String> = schema::identifiers_of(list, e).iter().map(|s| fold(s)).collect();
            if !ids.is_empty() && ids.iter().any(|i| eids.contains(i)) {
                return true;
            }
            if fold(&schema::title_of(list, e)) != title {
                return false;
            }
            let esub = fold(&schema::subtitle_of(list, e));
            // Si les deux fiches renseignent un sous-titre (ex: artiste/année pour un vinyle, ou date/lieu pour un concert),
            // elles ne sont en doublon que si leurs sous-titres correspondent.
            // Si la nouvelle fiche ou la fiche existante n'a pas précisé de sous-titre, le même titre suffit comme doublon.
            if !sub.is_empty() && !esub.is_empty() {
                sub == esub
            } else {
                true
            }
        })
    }

    /// Si un identifiant (numéro de set, code…) est présent sans nom, donne un nom provisoire (« Set 75436 »)
    /// que l'enrichissement remplacera par le vrai nom.
    fn ensure_provisional_title(list: &J, vals: &mut Map<String, Value>) -> bool {
        let title_key = js::gs(list, "titre");
        let id_val = js::str_list(&list["identifiant"])
            .into_iter()
            .find_map(|k| vals.get(&k).map(js::s))
            .filter(|s| !s.trim().is_empty());
        let Some(id_text) = id_val else { return false };
        let sing = js::capitalize(&js::gs(&list["noms"], "singulier"));
        let prov = if sing.is_empty() { id_text.clone() } else { format!("{sing} {id_text}") };
        let cur = vals.get(&title_key).map(js::s).unwrap_or_default();
        if cur.trim().is_empty() || fold(&cur) == fold(&id_text) || fold(&cur) == fold(&prov) {
            vals.insert(title_key, json!(prov));
            return true;
        }
        false
    }

    /// `opts` : { statut: "possede" | "souhaite", forcer: bool }.
    pub fn add_item(&self, list_id: &str, values: &J, opts: &J, source: &str) -> Result<J, String> {
        let before = self.get(list_id).ok_or("Liste introuvable.")?;
        let mut list = before.clone();
        let mut elements = js::arr(&list["elements"]);
        if elements.len() >= schema::MAX_ITEMS {
            return Err(format!("Cette liste est pleine ({} éléments au plus).", schema::MAX_ITEMS));
        }
        let mut vals = Map::new();
        Self::apply_values(&list, &mut vals, values)?;
        let mut prov = Self::ensure_provisional_title(&list, &mut vals) || js::gb(opts, "provisoire");
        // Code-barres scanné (site Nomade) sans nom : nom provisoire, remplacé par l'enrichissement.
        let barcode = clean_barcode(&js::gs(opts, "codeBarre"));
        if let Some(code) = &barcode {
            let title_key = js::gs(&list, "titre");
            if !vals.contains_key(&title_key) {
                vals.insert(title_key, json!(format!("Code-barres {code}")));
                prov = true;
            }
        }
        if let Some(key) = list["etat"].as_str().map(String::from) {
            if !vals.contains_key(&key) {
                let owned = match js::gs(opts, "statut").as_str() {
                    "souhaite" => false,
                    "possede" => true,
                    _ => schema::default_is_done(&list, &vals),
                };
                if let Some(label) = schema::statut_label(&list, owned) {
                    vals.insert(key, json!(label));
                }
            }
        }
        let title_key = js::gs(&list, "titre");
        if !vals.contains_key(&title_key) {
            return Err("Il faut au moins un nom pour ajouter un élément.".into());
        }
        Self::check_required(&list, &vals)?;
        if !js::gb(opts, "forcer") {
            let same_code = barcode.as_ref().and_then(|c| js::arr(&list["elements"]).into_iter().find(|e| js::gs(e, "codeBarre") == *c));
            if let Some(dup) = same_code.or_else(|| Self::find_duplicate(&list, &vals)) {
                return Ok(json!({ "ok": false, "doublon": true, "liste": list_id, "element": schema::public_item(&list, &dup),
                    "erreur": format!("« {} » est déjà dans la liste.", schema::title_of(&list, &dup)) }));
            }
        }
        let now = js::iso_now();
        let id = new_element_id();
        // Un fournisseur d'infos est branché : la fiche est un brouillon le temps de la recherche.
        // Liste sur mesure sans fournisseur : l'IA n'est interrogée que s'il manque des données.
        let can_enrich = (Self::has_provider(&list) || (Self::can_fill(&list) && Self::has_gaps(&list, &vals)))
            && self.o.upgrade().is_some()
            && !js::gb(opts, "sansEnrichissement");
        let mut item = json!({ "id": id, "creeLe": now, "majLe": now, "valeurs": Value::Object(vals), "source": source, "brouillon": can_enrich, "titreProvisoire": prov });
        if let Some(code) = &barcode {
            item["codeBarre"] = json!(code);
        }
        elements.push(item.clone());
        list["elements"] = json!(elements);
        self.save_list(&mut list)?;
        self.push_undo(&format!("ajout de {} à la liste {}", schema::title_of(&list, &item), js::gs(&list, "nom")), before);
        self.record("element_ajoute", json!({ "liste": list_id, "element": id, "source": source }));
        self.changed(list_id, Some(&id), "element_ajoute");
        if can_enrich {
            self.enrich_soon(list_id, &id, source, super::enrich::EnrichOpts::default());
        }
        Ok(json!({ "ok": true, "liste": list_id, "element": schema::public_item(&list, &item) }))
    }

    /// Demande à Claude Code (sans outil, modèle rapide) de découper et structurer le texte selon les champs de la liste.
    async fn format_text_with_ai(&self, list: &J, text: &str) -> Option<String> {
        let o = self.o.upgrade()?;
        if js::env("JARVIS_TEST").is_some() {
            return None;
        }
        let claude = o.claude_path()?;
        let fields: Vec<String> = js::arr(&list["champs"])
            .iter()
            .filter(|f| js::gs(f, "type") != "image")
            .map(|f| {
                let choix = if js::gs(f, "type") == "choix" { format!(" (choix : {})", js::str_list(&f["choix"]).join(" / ")) } else { String::new() };
                format!("- {} ({}{}) : {}", js::gs(f, "cle"), js::gs(f, "type"), choix, js::gs(f, "libelle"))
            })
            .collect();
        let ids = js::str_list(&list["identifiant"]).join(", ");
        let input = format!(
            "Liste : « {} » ({} / {})\nChamp titre : {}\nChamp identifiant (numéro/code) : {}\nChamps disponibles :\n{}\n- statut (optionnel : \"possede\" ou \"souhaite\" seulement si le texte l'indique)\n\nTexte de l'utilisateur à découper et mettre en forme :\n{}",
            js::gs(list, "nom"),
            js::gs(&list["noms"], "singulier"),
            js::gs(&list["noms"], "pluriel"),
            js::gs(list, "titre"),
            if ids.is_empty() { "aucun" } else { &ids },
            fields.join("\n"),
            text
        );
        let task_id = format!("bulk{}", base36(js::now_ms() as u64));
        o.emit(json!({ "type": "create", "id": task_id, "label": "Mise en forme (IA)", "detail": format!("Haiku · {}", js::gs(list, "nom")) }));

        use tokio::io::AsyncWriteExt;
        let mut cmd = tokio::process::Command::new(&claude);
        cmd.args([
            "-p", "--output-format", "text", "--tools", "", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}",
            "--permission-mode", "manual", "--no-session-persistence", "--system-prompt", BULK_PROMPT, "--model", "haiku",
        ])
        .current_dir(crate::paths::workspace())
        .env("JARVIS_TASK_ID", &task_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let res: Result<String, String> = async {
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(input.as_bytes()).await;
            }
            let out = tokio::time::timeout(std::time::Duration::from_secs(120), child.wait_with_output())
                .await
                .map_err(|_| "délai dépassé".to_string())?
                .map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && !stdout.is_empty() {
                Ok(stdout)
            } else {
                Err("réponse vide".to_string())
            }
        }
        .await;

        o.emit(json!({ "type": "finish", "id": task_id, "status": if res.is_ok() { "done" } else { "error" } }));
        res.ok()
    }

    /// Ajoute plusieurs éléments à partir d'un texte libre (mis en forme par l'IA avec repli et complétude locale).
    pub async fn add_from_text(&self, list_id: &str, a: &J, source: &str) -> Result<J, String> {
        let initial = self.get(list_id).ok_or("Liste introuvable.")?;
        let raw_text = js::slice_chars(js::gs(a, "texte").trim(), 20_000);
        if raw_text.is_empty() {
            return Err("Écris ou colle au moins un élément à ajouter.".into());
        }
        let (mut parsed_list, used_ai) = match self.format_text_with_ai(&initial, &raw_text).await {
            Some(ai_out) => {
                let ai_items = super::voice::parse_bulk_json(&initial, &ai_out, &raw_text);
                if ai_items.is_empty() {
                    (super::voice::parse_bulk_text(&initial, &raw_text), false)
                } else {
                    (ai_items, true)
                }
            }
            None => (super::voice::parse_bulk_text(&initial, &raw_text), false),
        };
        if used_ai {
            if let Some(id_key) = js::str_list(&initial["identifiant"]).into_iter().next() {
                for loc in super::voice::parse_bulk_text(&initial, &raw_text) {
                    if let Some(id_val) = loc.values.get(&id_key).map(js::s).filter(|s| !s.is_empty()) {
                        let already = parsed_list.iter().any(|p| p.values.get(&id_key).map(|v| fold(&js::s(v)) == fold(&id_val)).unwrap_or(false));
                        if !already {
                            parsed_list.push(loc);
                        }
                    }
                }
            }
        }

        self.insert_parsed(list_id, parsed_list, a, source, used_ai, "dans ce texte")
    }

    /// Importe un fichier CSV (export Excel ou tableur) : la 1re ligne donne les colonnes, retrouvées par le libellé ou la clé
    /// des champs de la liste ; les colonnes inconnues sont ignorées et signalées.
    pub fn import_csv(&self, list_id: &str, a: &J, source: &str) -> Result<J, String> {
        let list = self.get(list_id).ok_or("Liste introuvable.")?;
        let text = js::slice_chars(js::gs(a, "csv").trim_start_matches('\u{feff}'), 2_000_000);
        let rows = parse_csv(&text);
        let Some((header, body)) = rows.split_first() else { return Err("Le fichier CSV est vide.".into()) };
        let cols: Vec<Option<String>> = header
            .iter()
            .map(|h| {
                let f = fold(h);
                js::arr(&list["champs"]).iter().find(|c| fold(&js::gs(c, "cle")) == f || fold(&js::gs(c, "libelle")) == f).map(|c| js::gs(c, "cle"))
            })
            .collect();
        if !cols.iter().flatten().any(|k| *k == js::gs(&list, "titre")) {
            return Err(format!("Il manque la colonne « {} » (le nom de chaque fiche) dans la 1re ligne.", schema::field(&list, &js::gs(&list, "titre")).map(|f| js::gs(f, "libelle")).unwrap_or_default()));
        }
        let ignored: Vec<String> = header.iter().zip(&cols).filter(|(h, c)| c.is_none() && !h.trim().is_empty()).map(|(h, _)| h.trim().to_string()).collect();
        let parsed: Vec<super::voice::Parsed> = body
            .iter()
            .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
            .map(|r| {
                let mut p = super::voice::Parsed::default();
                for (cell, key) in r.iter().zip(&cols) {
                    if let (Some(k), true) = (key, !cell.trim().is_empty()) {
                        p.values.insert(k.clone(), json!(cell.trim()));
                    }
                }
                p
            })
            .collect();
        let mut out = self.insert_parsed(list_id, parsed, a, source, false, "dans ce fichier")?;
        out["colonnesIgnorees"] = json!(ignored);
        Ok(out)
    }

    /// Enregistre des éléments déjà découpés (texte libre ou CSV) : validation, doublons, un seul « annule », enrichissement.
    fn insert_parsed(&self, list_id: &str, parsed_list: Vec<super::voice::Parsed>, a: &J, source: &str, used_ai: bool, from: &str) -> Result<J, String> {
        let before = self.get(list_id).ok_or("Liste introuvable.")?;
        let mut list = before.clone();
        let mut elements = js::arr(&list["elements"]);
        if elements.len() >= schema::MAX_ITEMS {
            return Err(format!("Cette liste est pleine ({} éléments au plus).", schema::MAX_ITEMS));
        }

        let title_key = js::gs(&list, "titre");
        let etat_key = list["etat"].as_str().map(String::from);
        let default_statut = js::gs(a, "statut");
        let forcer = js::gb(a, "forcer");
        let allow_enrich = Self::has_provider(&list) && self.o.upgrade().is_some() && !js::gb(a, "sansEnrichissement");
        let now = js::iso_now();

        let mut added_items: Vec<J> = vec![];
        let mut enrich_ids: Vec<String> = vec![];
        let mut dup_count = 0usize;

        for p in parsed_list {
            if elements.len() >= schema::MAX_ITEMS {
                break;
            }
            let mut vals = Map::new();
            for (k, v) in &p.values {
                if let Some(f) = schema::field(&list, k) {
                    if let Ok(nv) = schema::normalize_value(f, v) {
                        if !nv.is_null() {
                            vals.insert(k.clone(), nv);
                        }
                    }
                }
            }
            let prov = Self::ensure_provisional_title(&list, &mut vals) || p.provisional;
            if !vals.contains_key(&title_key) {
                continue;
            }
            if let Some(key) = &etat_key {
                if !vals.contains_key(key) {
                    let st = p.statut.or_else(|| Some(default_statut.as_str()).filter(|s| !s.is_empty()));
                    let owned = match st {
                        Some("souhaite") => false,
                        Some("possede") => true,
                        _ => schema::default_is_done(&list, &vals),
                    };
                    if let Some(label) = schema::statut_label(&list, owned) {
                        vals.insert(key.clone(), json!(label));
                    }
                }
            }
            if !forcer && Self::find_duplicate(&list, &vals).is_some() {
                dup_count += 1;
                continue;
            }
            let id = format!("e{}{}_{}", base36(js::now_ms() as u64), js::rand_below(1000), added_items.len());
            let can_enrich = allow_enrich && enrich_ids.len() < 12;
            let item = json!({
                "id": id,
                "creeLe": now,
                "majLe": now,
                "valeurs": Value::Object(vals),
                "source": source,
                "brouillon": can_enrich,
                "titreProvisoire": prov
            });
            if can_enrich {
                enrich_ids.push(id);
            }
            elements.push(item.clone());
            list["elements"] = json!(elements.clone());
            added_items.push(item);
        }

        if added_items.is_empty() && dup_count == 0 {
            return Err(format!("Je n'ai trouvé aucun élément à ajouter {from}."));
        }

        let sing = js::gs(&list["noms"], "singulier");
        let plur = js::gs(&list["noms"], "pluriel");
        let n = added_items.len();

        if n > 0 {
            self.save_list(&mut list)?;
            let noun = if n > 1 { &plur } else { &sing };
            self.push_undo(&format!("ajout de {n} {noun} à la liste {}", js::gs(&list, "nom")), before);
            self.record("elements_ajoutes_texte", json!({ "liste": list_id, "ajoutes": n, "doublons": dup_count, "ia": used_ai, "source": source }));
            self.changed(list_id, None, "elements_ajoutes");
            if !enrich_ids.is_empty() {
                if let Some(o) = self.o.upgrade() {
                    let lid = list_id.to_string();
                    let src = source.to_string();
                    js::spawn(async move {
                        for eid in enrich_ids {
                            let _ = o.collections.enrich(&lid, &eid, &src, super::enrich::EnrichOpts::default()).await;
                        }
                    });
                }
            }
        }

        let dup_note = if dup_count > 0 {
            format!(" ({dup_count} doublon{} ignoré{})", if dup_count > 1 { "s" } else { "" }, if dup_count > 1 { "s" } else { "" })
        } else {
            String::new()
        };
        let resume = if n == 0 {
            format!("Tous les éléments sont déjà dans la liste{dup_note}.")
        } else {
            format!("{n} {} ajouté{}{dup_note}.", if n > 1 { &plur } else { &sing }, if n > 1 { "s" } else { "" })
        };
        let public_items: Vec<J> = added_items.iter().map(|i| schema::public_item(&list, i)).collect();
        Ok(json!({
            "ok": true,
            "liste": list_id,
            "ajoutes": n,
            "doublons": dup_count,
            "ia": used_ai,
            "elements": public_items,
            "resume": resume
        }))
    }

    /// Champs obligatoires (schéma › champs › obligatoire) absents des valeurs : refus, avec leur nom.
    fn check_required(list: &J, vals: &Map<String, Value>) -> Result<(), String> {
        let missing: Vec<String> = js::arr(&list["champs"])
            .iter()
            .filter(|f| js::gb(f, "obligatoire") && vals.get(&js::gs(f, "cle")).map_or(true, |v| v.is_null() || v.as_str() == Some("")))
            .map(|f| js::gs(f, "libelle"))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!("Champ obligatoire à remplir : {}.", missing.join(", ")))
        }
    }

    pub fn update_item(&self, list_id: &str, element_id: &str, values: &J, source: &str) -> Result<J, String> {
        let before = self.get(list_id).ok_or("Liste introuvable.")?;
        let mut list = before.clone();
        let mut elements = js::arr(&list["elements"]);
        let item = elements.iter_mut().find(|e| js::gs(e, "id") == element_id).ok_or("Élément introuvable.")?;
        let mut vals = item["valeurs"].as_object().cloned().unwrap_or_default();
        Self::apply_values(&list, &mut vals, values)?;
        if !vals.contains_key(&js::gs(&list, "titre")) {
            return Err("Le nom ne peut pas être vide.".into());
        }
        Self::check_required(&list, &vals)?;
        item["valeurs"] = Value::Object(vals);
        item["majLe"] = json!(js::iso_now());
        item["brouillon"] = json!(false);
        let updated = item.clone();
        list["elements"] = json!(elements);
        self.save_list(&mut list)?;
        self.push_undo(&format!("modification de {}", schema::title_of(&list, &updated)), before);
        self.record("element_modifie", json!({ "liste": list_id, "element": element_id, "champs": values.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(), "source": source }));
        self.changed(list_id, Some(element_id), "element_modifie");
        Ok(json!({ "ok": true, "liste": list_id, "element": schema::public_item(&list, &updated) }))
    }

    pub fn delete_item(&self, list_id: &str, element_id: &str, source: &str) -> Result<J, String> {
        let before = self.get(list_id).ok_or("Liste introuvable.")?;
        let mut list = before.clone();
        let mut elements = js::arr(&list["elements"]);
        let idx = elements.iter().position(|e| js::gs(e, "id") == element_id).ok_or("Élément introuvable.")?;
        let removed = elements.remove(idx);
        list["elements"] = json!(elements);
        self.save_list(&mut list)?;
        let title = schema::title_of(&list, &removed);
        self.push_undo(&format!("retrait de {title} de la liste {}", js::gs(&list, "nom")), before);
        self.record("element_supprime", json!({ "liste": list_id, "element": element_id, "source": source }));
        self.changed(list_id, Some(element_id), "element_supprime");
        Ok(json!({ "ok": true, "liste": list_id, "titre": title }))
    }

    fn selected_ids(a: &J) -> Vec<String> {
        let mut ids = js::str_list(&a["elements"]);
        ids.sort();
        ids.dedup();
        ids
    }

    /// Retire plusieurs fiches d'un coup ; un seul « annule » les remet toutes.
    pub fn delete_items(&self, list_id: &str, a: &J, source: &str) -> Result<J, String> {
        let before = self.get(list_id).ok_or("Liste introuvable.")?;
        let ids = Self::selected_ids(a);
        if ids.is_empty() {
            return Err("Aucun élément sélectionné.".into());
        }
        let mut list = before.clone();
        let mut elements = js::arr(&list["elements"]);
        let n0 = elements.len();
        elements.retain(|e| !ids.contains(&js::gs(e, "id")));
        let n = n0 - elements.len();
        if n == 0 {
            return Err("Éléments introuvables.".into());
        }
        list["elements"] = json!(elements);
        self.save_list(&mut list)?;
        self.push_undo(&format!("retrait de {n} éléments de la liste {}", js::gs(&list, "nom")), before);
        self.record("elements_supprimes", json!({ "liste": list_id, "nombre": n, "source": source }));
        self.changed(list_id, None, "element_supprime");
        Ok(json!({ "ok": true, "liste": list_id, "supprimes": n }))
    }

    /// Applique les mêmes valeurs à plusieurs fiches ; un seul « annule » rétablit tout.
    pub fn update_items(&self, list_id: &str, a: &J, source: &str) -> Result<J, String> {
        let before = self.get(list_id).ok_or("Liste introuvable.")?;
        let ids = Self::selected_ids(a);
        if ids.is_empty() {
            return Err("Aucun élément sélectionné.".into());
        }
        let values = &a["valeurs"];
        if values.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            return Err("Aucune valeur à appliquer.".into());
        }
        let title_key = js::gs(&before, "titre");
        if values.get(&title_key).is_some() && ids.len() > 1 {
            return Err("Le nom ne peut pas être le même pour plusieurs fiches.".into());
        }
        let mut list = before.clone();
        let mut elements = js::arr(&list["elements"]);
        let mut n = 0;
        for item in elements.iter_mut().filter(|e| ids.contains(&js::gs(e, "id"))) {
            let mut vals = item["valeurs"].as_object().cloned().unwrap_or_default();
            Self::apply_values(&list, &mut vals, values)?;
            if !vals.contains_key(&title_key) {
                return Err("Le nom ne peut pas être vide.".into());
            }
            item["valeurs"] = Value::Object(vals);
            item["majLe"] = json!(js::iso_now());
            item["brouillon"] = json!(false);
            n += 1;
        }
        if n == 0 {
            return Err("Éléments introuvables.".into());
        }
        list["elements"] = json!(elements);
        self.save_list(&mut list)?;
        self.push_undo(&format!("modification de {n} éléments de la liste {}", js::gs(&list, "nom")), before);
        self.record("elements_modifies", json!({ "liste": list_id, "nombre": n, "champs": values.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(), "source": source }));
        self.changed(list_id, None, "element_modifie");
        Ok(json!({ "ok": true, "liste": list_id, "modifies": n }))
    }

    /// « J'ai acheté / je le voudrais » : change le statut, avec prix, lieu et date éventuels.
    pub fn set_status(&self, list_id: &str, element_id: &str, owned: bool, extra: &J, source: &str) -> Result<J, String> {
        let list = self.get(list_id).ok_or("Liste introuvable.")?;
        let key = list["etat"].as_str().ok_or("Cette liste n'a pas de champ de statut.")?.to_string();
        let label = schema::statut_label(&list, owned).ok_or("Le champ de statut n'a pas de choix « possédé / souhaité ».")?;
        let mut values = extra.as_object().cloned().unwrap_or_default();
        values.insert(key, json!(label));
        let date_key = schema::vocab(&list)["dateFait"].as_str().map(String::from).filter(|k| schema::field(&list, k).is_some());
        if let (true, Some(dk)) = (owned, date_key.as_ref()) {
            if !values.contains_key(dk) {
                let item = js::arr(&list["elements"]).into_iter().find(|e| js::gs(e, "id") == element_id);
                if item.map(|i| schema::value(&i, dk).is_null()).unwrap_or(false) {
                    values.insert(dk.clone(), json!(js::local_now().format("%Y-%m-%d").to_string()));
                }
            }
        }
        self.update_item(list_id, element_id, &Value::Object(values), source)
    }

    // --- Recherche et export ---------------------------------------------------------------------

    pub async fn search(&self, query: &str, only: Option<&str>, limit: usize) -> Vec<(J, J, Hit)> {
        let passages = self.passages(only);
        let hits = self.index.search(query, &passages, limit).await;
        hits.into_iter()
            .filter_map(|h| {
                let list = self.get(&h.list)?;
                let item = js::arr(&list["elements"]).into_iter().find(|e| js::gs(e, "id") == h.element)?;
                Some((list, item, h))
            })
            .collect()
    }

    pub fn export_csv(&self, list_id: &str) -> Result<String, String> {
        let list = self.get(list_id).ok_or("Liste introuvable.")?;
        let fields = js::arr(&list["champs"]);
        let cell = |s: &str| format!("\"{}\"", s.replace('"', "\"\"").replace('\n', " "));
        let mut lines = vec![fields.iter().map(|f| cell(&js::gs(f, "libelle"))).collect::<Vec<_>>().join(";")];
        for item in js::arr(&list["elements"]) {
            lines.push(fields.iter().map(|f| cell(&schema::display(&list, &item, &js::gs(f, "cle")))).collect::<Vec<_>>().join(";"));
        }
        Ok(lines.join("\r\n"))
    }

    pub fn stats(&self) -> J {
        let passages = self.passages(None).len();
        let mut s = self.index.stats(passages);
        js::assign(&mut s, json!({ "listes": self.count(), "elements": self.item_count() }));
        s
    }

    // --- Affichage dans le HUD -----------------------------------------------------------------

    pub fn show_list(&self, list_id: &str) -> Result<J, String> {
        let list = self.get(list_id).ok_or("Liste introuvable.")?;
        let o = self.o.up();
        o.show_widget("tool", cards::list_card(&list));
        Ok(json!({ "ok": true, "titre": list["nom"] }))
    }

    pub fn show_item(&self, list_id: &str, element_id: &str) -> Result<J, String> {
        let list = self.get(list_id).ok_or("Liste introuvable.")?;
        if !js::arr(&list["elements"]).iter().any(|e| js::gs(e, "id") == element_id) {
            return Err("Élément introuvable.".into());
        }
        self.open_items.borrow_mut().insert(list_id.to_string(), element_id.to_string());
        self.o.up().show_widget("tool", cards::item_card(&list, element_id));
        Ok(json!({ "ok": true }))
    }

    pub fn show_overview(&self) {
        let o = self.o.up();
        o.show_widget("tool", cards::overview_card(&o));
    }

    // --- Point d'entrée commun -----------------------------------------------------------------

    /// `source` : « voix », « card » ou « aide » (inscrit au journal, jamais les valeurs saisies).
    pub async fn dispatch(&self, op: &str, a: &J, source: &str) -> J {
        let list_id = js::gs(a, "liste");
        let element_id = js::gs(a, "element");
        let r: Result<J, String> = match op {
            "lister" => Ok(json!({ "listes": self.all().iter().map(|l| schema::public_list(l, false)).collect::<Vec<_>>() })),
            "lire" => self.get(&list_id).map(|l| json!({ "liste": schema::public_list(&l, true) })).ok_or_else(|| "Liste introuvable.".to_string()),
            "modeles" => Ok(json!({ "modeles": self.all_models() })),
            "creer_liste" => self.create_list(a, source).map(|l| json!({ "liste": schema::public_list(&l, false) })),
            "modifier_schema" => self.update_schema(&list_id, a, source).map(|l| json!({ "liste": schema::public_list(&l, false) })),
            "supprimer_liste" => self.delete_list(&list_id, a["confirmer"].as_str(), source),
            "ajouter_element" => self.add_item(&list_id, &a["valeurs"], a, source),
            "ajouter_texte" => self.add_from_text(&list_id, a, source).await,
            "supprimer_elements" => self.delete_items(&list_id, a, source),
            "modifier_elements" => self.update_items(&list_id, a, source),
            "modifier_element" => self.update_item(&list_id, &element_id, &a["valeurs"], source),
            "supprimer_element" => self.delete_item(&list_id, &element_id, source),
            "changer_statut" => self.set_status(&list_id, &element_id, js::gs(a, "statut") != "souhaite", &a["valeurs"], source),
            "exporter_csv" => self.export_csv(&list_id).map(|csv| json!({ "csv": csv })),
            "reindexer" => {
                self.index.reset();
                let passages = self.passages(None);
                match self.index.ensure(&passages).await {
                    Ok(n) => Ok(json!({ "calcules": n, "stats": self.stats() })),
                    Err(e) => Err(format!("Réindexation impossible : {e}")),
                }
            }
            "rechercher" => {
                let limit = a["limite"].as_u64().unwrap_or(8).min(30) as usize;
                let only = Some(list_id.as_str()).filter(|s| !s.is_empty());
                let rows = self.search(&js::gs(a, "requete"), only, limit).await;
                Ok(json!({ "resultats": rows.iter().map(|(l, i, h)| json!({
                    "liste": l["id"], "listeNom": l["nom"], "vocabulaire": schema::vocab(l), "element": schema::public_item(l, i), "score": (h.score * 100.0).round() / 100.0, "exact": h.exact
                })).collect::<Vec<_>>() }))
            }
            "stats" => Ok(self.stats()),
            "enrichir" => self.enrich(&list_id, &element_id, source, super::enrich::EnrichOpts { ecraser: js::gb(a, "ecraser"), suivant: js::gb(a, "suivant") }).await,
            "importer_csv" => self.import_csv(&list_id, a, source),
            "statistiques" => self.get(&list_id).map(|l| json!({ "stats": super::query::statistics(&l) })).ok_or_else(|| "Liste introuvable.".to_string()),
            "enrichir_liste" => self.enrich_list(&list_id, a, source),
            "action" => self.run_action(&list_id, &element_id, &js::gs(a, "action")).await,
            "ouvrir_lien_prix" => self.open_price_link(&list_id, &element_id, a["index"].as_u64().unwrap_or(u64::MAX) as usize).await,
            // Pilotage du HUD (boutons de la card)
            "ouvrir" if element_id.is_empty() && list_id.is_empty() => {
                self.show_overview();
                Ok(json!({}))
            }
            "ouvrir" if element_id.is_empty() => self.show_list(&list_id),
            "ouvrir" => self.show_item(&list_id, &element_id),
            "proposer" => self.propose(a).await,
            "annuler_creation" => {
                self.cancel_conversation();
                Ok(json!({}))
            }
            "creer_depuis_proposition" => self.create_from_conversation(a, source),
            _ => Err(format!("Opération inconnue : {op}")),
        };
        match r {
            Ok(mut v) => {
                if let Some(m) = v.as_object_mut() {
                    m.entry("ok").or_insert(json!(true));
                }
                v
            }
            Err(e) => json!({ "ok": false, "erreur": e }),
        }
    }
}

/// Statut lisible pour la voix.
pub fn statut_phrase(list: &J, s: Statut) -> String {
    let voc = schema::vocab(list);
    match s {
        Statut::Possede => {
            let yes = js::gs(&voc, "ouiPhrase");
            let t = yes.trim_start_matches("Oui, ").to_string();
            if t.is_empty() { "tu l'as".into() } else { t }
        }
        Statut::Souhaite => format!("il {}", js::gs(&voc, "vouluPhrase")),
        Statut::Inconnu => "il est dans ta liste".into(),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn identifiants_et_pluriels() {
        assert!(valid_id("vinyles_lx4k2"));
        assert!(!valid_id("../secret"));
        assert!(!valid_id("A"));
        assert!(!valid_id(""));
        assert_eq!(loose("Disques vinyles"), loose("disque vinyle"));
        assert_eq!(loose("lego"), "lego");
    }

    fn vinyl() -> J {
        let mut s = schema::normalize_schema(&schema::preset("vinyle").unwrap()).unwrap();
        s["id"] = json!("vinyles_test");
        s["elements"] = json!([
            { "id": "e1", "valeurs": { "titre": "Under a Blood Red Sky", "artiste": "U2", "etat": "Possédé" } },
            { "id": "e2", "valeurs": { "titre": "The Joshua Tree", "artiste": "U2", "etat": "Souhaité" } }
        ]);
        s
    }

    fn service() -> (Collections, PathBuf) {
        let dir = std::env::temp_dir().join(format!("jarvis-test-collections-{}-{}", std::process::id(), js::rand_below(1_000_000)));
        let svc = Collections::new(std::rc::Weak::<crate::orch::Orchestrator>::new(), dir.clone(), Rc::new(|_| {}));
        (svc, dir)
    }

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        tokio::task::LocalSet::new().block_on(&rt, f)
    }

    #[test]
    fn modele_personnalise_et_origine_invite() {
        run(async { modele_personnalise_et_origine_invite_impl() });
    }

    fn modele_personnalise_et_origine_invite_impl() {
        let base = std::env::temp_dir().join(format!("jarvis-test-modeles-{}-{}", std::process::id(), js::rand_below(1_000_000)));
        let dir = base.join("collections");
        let svc = Collections::new(std::rc::Weak::<crate::orch::Orchestrator>::new(), dir, Rc::new(|_| {}));
        let animaux = json!({
            "nom": "Mes animaux", "alias": ["animaux", "bêtes"],
            "champs": [{ "cle": "nom", "libelle": "Nom", "type": "texte" }, { "cle": "espece", "libelle": "Espèce", "type": "texte" }, { "cle": "etat", "libelle": "Statut", "type": "choix", "choix": ["Vivant", "Disparu"] }],
            "titre": "nom", "etat": "etat"
        });
        let model = svc.save_custom_model(&animaux).unwrap();
        assert_eq!(js::gs(&model, "id"), "perso_mes_animaux");
        assert_eq!(svc.custom_models().len(), 1);
        assert!(svc.find_custom_model("une liste de bêtes de la maison").is_some());
        assert!(svc.find_custom_model("films").is_none());
        // Même nom : le modèle est remplacé, pas dupliqué ; les modèles livrés restent présents.
        svc.save_custom_model(&animaux).unwrap();
        assert_eq!(svc.custom_models().len(), 1);
        assert_eq!(svc.all_models().len(), schema::presets().len() + 1);
        // La liste porte son origine, qui survit à une modification de schéma et part dans la fiche publique.
        let origine = json!({ "type": "invite", "login": "marie", "nom": "Marie", "relation": "famille", "depotId": 12 });
        let list = svc.create_list_from(&model, "nomade", Some(origine)).unwrap();
        let id = js::gs(&list, "id");
        assert_eq!(schema::public_list(&list, false)["origine"]["login"], "marie");
        let after = svc.update_schema(&id, &json!({ "nom": "Mes bêtes" }), "test").unwrap();
        assert_eq!(after["origine"]["type"], "invite");
        assert_eq!(svc.get(&id).unwrap()["origine"]["nom"], "Marie");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn concert_sans_date_refuse_et_tri_par_date() {
        let (svc, dir) = service();
        let d = |op: &str, a: J| run(svc.dispatch(op, &a, "test"));
        let created = d("creer_liste", schema::preset("concert").unwrap());
        let id = js::gs(&created["liste"], "id");
        assert!(!id.is_empty(), "{created}");
        assert_eq!(svc.get(&id).unwrap()["tri"], "date_desc");
        // La date du concert est obligatoire, à l'ajout comme à la modification.
        let r = d("ajouter_element", json!({ "liste": id, "valeurs": { "artiste": "U2" } }));
        assert!(!js::gb(&r, "ok") && js::gs(&r, "erreur").contains("Date du concert"), "{r}");
        let r = d("ajouter_element", json!({ "liste": id, "valeurs": { "artiste": "U2", "date": "1997-09-06" }, "sansEnrichissement": true }));
        assert!(js::gb(&r, "ok"), "{r}");
        let el = js::gs(&r["element"], "id");
        let r = d("modifier_element", json!({ "liste": id, "element": el, "valeurs": { "date": "" } }));
        assert!(!js::gb(&r, "ok"), "{r}");
        // Rendue facultative depuis l'éditeur de la liste : l'ajout sans date passe.
        d("modifier_schema", json!({ "liste": id, "obligatoire": { "date": false } }));
        let r = d("ajouter_element", json!({ "liste": id, "valeurs": { "artiste": "M" }, "sansEnrichissement": true }));
        assert!(js::gb(&r, "ok"), "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn csv_lu_avec_guillemets_et_separateur() {
        let rows = parse_csv("titre;artiste\n\"Un; titre\";\"Le \"\"groupe\"\"\"\r\nAutre;X\n");
        assert_eq!(rows, vec![vec!["titre", "artiste"], vec!["Un; titre", "Le \"groupe\""], vec!["Autre", "X"]]);
    }

    #[test]
    fn import_d_un_csv_dans_une_liste() {
        let (svc, dir) = service();
        let d = |op: &str, a: J| run(svc.dispatch(op, &a, "test"));
        let created = d("creer_liste", schema::preset("vinyle").unwrap());
        let id = js::gs(&created["liste"], "id");
        let r = d("importer_csv", json!({ "liste": id, "csv": "titre;artiste;annee;couleur\nThriller;Michael Jackson;1982;rouge\nUnder a Blood Red Sky;U2;1983;bleu\nThriller;Michael Jackson;1982;rouge\n", "sansEnrichissement": true }));
        assert!(js::gb(&r, "ok"), "{r}");
        assert_eq!(r["ajoutes"], json!(2), "{r}");
        assert_eq!(r["doublons"], json!(1), "{r}");
        assert_eq!(r["colonnesIgnorees"], json!(["couleur"]));
        assert_eq!(d("importer_csv", json!({ "liste": id, "csv": "artiste\nU2\n" }))["ok"], json!(false), "sans colonne du nom, refusé");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cycle_complet_creer_ajouter_modifier_supprimer() {
        let (svc, dir) = service();
        let d = |op: &str, a: J| run(svc.dispatch(op, &a, "test"));

        let created = d("creer_liste", schema::preset("vinyle").unwrap());
        assert!(js::gb(&created, "ok"), "{created}");
        let id = js::gs(&created["liste"], "id");
        assert!(id.starts_with("vinyles_"), "{id}");
        assert!(d("creer_liste", schema::preset("vinyle").unwrap())["ok"] == json!(false), "un nom déjà pris est refusé");

        // Ajout : le statut par défaut est « possédé ».
        let added = d("ajouter_element", json!({ "liste": id, "valeurs": { "titre": "Under a Blood Red Sky", "artiste": "U2", "prix_achat": "18 €", "date_achat": "12/03/2024" } }));
        assert!(js::gb(&added, "ok"), "{added}");
        assert_eq!(added["element"]["statut"], json!("possede"));
        assert_eq!(added["element"]["vignette"], json!("18 €"));
        let eid = js::gs(&added["element"], "id");

        // Doublon, valeur invalide, champ inconnu, nom manquant.
        let dup = d("ajouter_element", json!({ "liste": id, "valeurs": { "titre": "under a blood red sky", "artiste": "u2" } }));
        assert_eq!(dup["ok"], json!(false));
        assert_eq!(dup["doublon"], json!(true));
        assert!(d("ajouter_element", json!({ "liste": id, "valeurs": { "titre": "X", "prix_achat": "beaucoup" } }))["ok"] == json!(false));
        assert!(d("ajouter_element", json!({ "liste": id, "valeurs": { "titre": "X", "secret": "1" } }))["ok"] == json!(false));
        assert!(d("ajouter_element", json!({ "liste": id, "valeurs": { "artiste": "U2" } }))["ok"] == json!(false));
        assert!(d("ajouter_element", json!({ "liste": "../../etc/passwd", "valeurs": { "titre": "X" } }))["ok"] == json!(false));

        // Un disque souhaité, puis acheté.
        let wish = d("ajouter_element", json!({ "liste": id, "statut": "souhaite", "valeurs": { "titre": "The Joshua Tree", "artiste": "U2" } }));
        assert_eq!(wish["element"]["statut"], json!("souhaite"));
        let wid = js::gs(&wish["element"], "id");
        let bought = d("changer_statut", json!({ "liste": id, "element": wid, "statut": "possede", "valeurs": { "prix_achat": 22.5, "lieu_achat": "Cultura" } }));
        assert!(js::gb(&bought, "ok"), "{bought}");
        assert_eq!(bought["element"]["statut"], json!("possede"));
        assert!(bought["element"]["valeurs"]["date_achat"].is_string(), "la date d'achat est remplie");

        // Modification, recherche, export.
        let upd = d("modifier_element", json!({ "liste": id, "element": eid, "valeurs": { "annee": 1983, "lieu_achat": "Fnac Strasbourg" } }));
        assert_eq!(upd["element"]["affichage"]["annee"], json!("1983"));
        // Modification et suppression groupées : un seul « annule » défait tout.
        let both = json!([eid, wid]);
        let bulk = d("modifier_elements", json!({ "liste": id, "elements": both, "valeurs": { "notes": "Écouté en boucle" } }));
        assert_eq!(bulk["modifies"], json!(2), "{bulk}");
        assert!(d("modifier_elements", json!({ "liste": id, "elements": both, "valeurs": { "titre": "Même nom" } }))["ok"] == json!(false));
        assert!(d("modifier_elements", json!({ "liste": id, "elements": [], "valeurs": { "annee": 1 } }))["ok"] == json!(false));
        let tmp = d("ajouter_element", json!({ "liste": id, "valeurs": { "titre": "Provisoire", "artiste": "Test" } }));
        let tid = js::gs(&tmp["element"], "id");
        let gone = d("supprimer_elements", json!({ "liste": id, "elements": [tid] }));
        assert_eq!(gone["supprimes"], json!(1), "{gone}");
        assert_eq!(d("supprimer_elements", json!({ "liste": id, "elements": [tid] }))["ok"], json!(false));
        let found = d("rechercher", json!({ "requete": "blood red sky", "limite": 3 }));
        assert_eq!(found["resultats"][0]["element"]["id"], json!(eid));
        let csv = d("exporter_csv", json!({ "liste": id }));
        assert!(js::gs(&csv, "csv").contains("Under a Blood Red Sky") && js::gs(&csv, "csv").contains("\"Fnac Strasbourg\""));

        // Schéma : un champ de plus, un champ de moins ; le nom ne peut pas être retiré.
        let sch = d("modifier_schema", json!({ "liste": id, "ajouter": [{ "libelle": "Pays", "type": "texte" }], "retirer": ["label"] }));
        assert!(js::gb(&sch, "ok"), "{sch}");
        assert!(schema::field(&svc.get(&id).unwrap(), "pays").is_some());
        assert!(schema::field(&svc.get(&id).unwrap(), "label").is_none());
        assert_eq!(d("modifier_schema", json!({ "liste": id, "retirer": ["titre"] }))["ok"], json!(false));

        // Tout est retrouvé après un redémarrage.
        let again = Collections::new(std::rc::Weak::<crate::orch::Orchestrator>::new(), dir.clone(), Rc::new(|_| {}));
        assert_eq!(again.count(), 1);
        assert_eq!(again.item_count(), 2);

        // Suppressions.
        assert!(js::gb(&d("supprimer_element", json!({ "liste": id, "element": wid })), "ok"));
        assert_eq!(svc.item_count(), 1);
        assert_eq!(d("supprimer_liste", json!({ "liste": id, "confirmer": "autre chose" }))["ok"], json!(false), "il faut retaper le nom");
        assert_eq!(svc.count(), 1);
        assert!(js::gb(&d("supprimer_liste", json!({ "liste": id, "confirmer": "vinyles" })), "ok"));
        assert_eq!(svc.count(), 0);
        assert!(!dir.join(format!("{id}.json")).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retrouver_une_liste_par_ce_que_dit_l_utilisateur() {
        let (svc, dir) = service();
        for m in ["vinyle", "lego"] {
            let r = run(svc.dispatch("creer_liste", &schema::preset(m).unwrap(), "test"));
            assert!(js::gb(&r, "ok"), "{r}");
        }
        let name = |s: &str| svc.find_by_phrase(s).map(|l| js::gs(&l, "nom"));
        assert_eq!(name("ma liste de disques").as_deref(), Some("Vinyles"));
        assert_eq!(name("mes vinyles").as_deref(), Some("Vinyles"));
        assert_eq!(name("disque vinyl").as_deref(), Some("Vinyles"));
        assert_eq!(name("lego").as_deref(), Some("Lego"));
        assert_eq!(name("mes legos").as_deref(), Some("Lego"));
        assert_eq!(name("courses"), None, "une liste à cocher n'est pas une liste à fiches");
        let (list, rest) = svc.find_by_prefix("vinyl le disque Under a Blood Red Sky du groupe U2").unwrap();
        assert_eq!(js::gs(&list, "nom"), "Vinyles");
        assert_eq!(rest, "le disque Under a Blood Red Sky du groupe U2");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn detection_des_doublons() {
        let l = vinyl();
        let v = |t: &str, a: &str| {
            let mut m = Map::new();
            m.insert("titre".into(), json!(t));
            if !a.is_empty() {
                m.insert("artiste".into(), json!(a));
            }
            m
        };
        assert!(Collections::find_duplicate(&l, &v("under a blood red sky", "u2")).is_some());
        assert!(Collections::find_duplicate(&l, &v("Under a Blood Red Sky", "")).is_some(), "sans artiste, le titre suffit");
        assert!(Collections::find_duplicate(&l, &v("Under a Blood Red Sky", "Autre groupe")).is_none());
        assert!(Collections::find_duplicate(&l, &v("War", "U2")).is_none());
    }

    #[test]
    fn ajout_en_texte_lego_et_vinyles() {
        let (svc, dir) = service();
        let d = |op: &str, a: J| run(svc.dispatch(op, &a, "test"));

        let lego = d("creer_liste", schema::preset("lego").unwrap());
        let lid = js::gs(&lego["liste"], "id");

        // Liste de numéros de sets (séparés par espaces, virgules et retours à la ligne) + doublon dans le même texte.
        let r = d("ajouter_texte", json!({ "liste": lid, "texte": "Voici mes sets : 75192, 75436 10307\n42156 - Porsche 963\n75436" }));
        assert!(js::gb(&r, "ok"), "{r}");
        assert_eq!(r["ajoutes"], json!(4));
        assert_eq!(r["doublons"], json!(1));
        let full = svc.get(&lid).unwrap();
        let elts = js::arr(&full["elements"]);
        assert_eq!(elts.len(), 4);
        assert_eq!(elts[0]["valeurs"]["numero"], json!("75192"));
        assert_eq!(elts[0]["valeurs"]["nom"], json!("Set 75192"));
        assert_eq!(elts[0]["titreProvisoire"], json!(true));
        assert_eq!(elts[3]["valeurs"]["numero"], json!("42156"));
        assert_eq!(elts[3]["valeurs"]["nom"], json!("Porsche 963"));
        assert_eq!(elts[3]["titreProvisoire"], json!(false));

        // Un numéro seul via ajouter_element reçoit aussi un nom provisoire.
        let single = d("ajouter_element", json!({ "liste": lid, "valeurs": { "numero": "10316" } }));
        assert!(js::gb(&single, "ok"), "{single}");
        assert_eq!(single["element"]["titre"], json!("Set 10316"));

        // Lecture d'une réponse JSON de l'IA.
        let parsed = super::super::voice::parse_bulk_json(
            &full,
            "```json\n[{\"numero\": \"75367\", \"statut\": \"souhaite\"}, {\"numero\": \"10497\", \"nom\": \"Galaxy Explorer\", \"prix_achat\": 99.99}]\n```",
            "75367 souhaité, 10497 Galaxy Explorer 99,99 €",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].values.get("numero"), Some(&json!("75367")));
        assert_eq!(parsed[0].values.get("nom"), Some(&json!("Set 75367")));
        assert!(parsed[0].provisional);
        assert_eq!(parsed[0].statut, Some("souhaite"));
        assert_eq!(parsed[1].values.get("nom"), Some(&json!("Galaxy Explorer")));
        // Test de parsing d'une liste de concerts avec setlist/notes
        let concert_schema = schema::normalize_schema(&schema::preset("concert").unwrap()).unwrap();
        let parsed_concerts = super::super::voice::parse_bulk_json(
            &concert_schema,
            r#"[
                {
                    "artiste": "a-ha",
                    "date": "1988-08-27",
                    "salle": "Stade en plein air",
                    "ville": "Royan",
                    "notes": "Tournée : Stay on These Roads Tour. Affluence : 10 000 à 15 000 spectateurs.\nSetlist :\n- Take On Me\n- The Sun Always Shines on T.V.\n- Hunting High and Low"
                },
                {
                    "artiste": "Deluxe",
                    "date": "2026-10-10",
                    "salle": "Dock des Suds",
                    "ville": "Marseille",
                    "notes": "Fiesta des Suds"
                }
            ]"#,
            "27 août 1988 a-ha Royan",
        );
        assert_eq!(parsed_concerts.len(), 2);
        assert_eq!(parsed_concerts[0].values.get("artiste"), Some(&json!("a-ha")));
        assert_eq!(parsed_concerts[0].values.get("date"), Some(&json!("1988-08-27")));
        assert_eq!(parsed_concerts[0].values.get("salle"), Some(&json!("Stade en plein air")));
        assert_eq!(parsed_concerts[0].values.get("ville"), Some(&json!("Royan")));
        assert!(parsed_concerts[0].values.get("notes").unwrap().as_str().unwrap().contains("Take On Me"));

        let _ = std::fs::remove_dir_all(dir);
    }
}
