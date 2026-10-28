// Listes intelligentes (orchestrator/smart-lists.js) : courses, valise, bricolage, tâches, cadeaux…
// Création assistée (questions de précision), génération structurée, mémorisation dans la mémoire
// sémantique (épingle « list_<id> »), card interactive (cocher, ajouter, copier, supprimer).
// Une liste = data/lists/<id>.json.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::automations::base36;
use super::intent::Intent;
use super::{OrchRef, Up};
use crate::js::{self, J};

pub struct ListType {
    pub label: &'static str,
    pub icon: &'static str,
    pub questions: &'static [&'static str],
    pub default_categories: &'static [&'static str],
}

pub fn list_type(t: &str) -> &'static ListType {
    const COURSES: ListType = ListType {
        label: "Courses & Alimentation",
        icon: "🛒",
        questions: &["Pour combien de personnes et pour quelle durée ou occasion ?", "As-tu des restrictions ou préférences (végétarien, sans gluten, bio, envie particulière) ?"],
        default_categories: &["Fruits & Légumes", "Frais & Produits laitiers", "Épicerie & Féculents", "Boissons", "Entretien & Hygiène"],
    };
    const VOYAGE: ListType = ListType {
        label: "Valise & Voyage",
        icon: "✈️",
        questions: &["Pour quelle destination et combien de jours pars-tu ?", "Quel type de climat ou météo est prévu (chaud, froid, pluie) ?", "Voyages-tu avec un bagage cabine ou une valise en soute ?"],
        default_categories: &["Vêtements", "Trousse de toilette", "Papiers & Billets", "High-tech & Chargeurs", "Pratique & Santé"],
    };
    const BRICOLAGE: ListType = ListType {
        label: "Bricolage & Travaux",
        icon: "🔨",
        questions: &["Quel est le projet ou les travaux à réaliser ?", "As-tu déjà des outils ou faut-il prévoir outillage et consommables ?"],
        default_categories: &["Matériaux", "Quincaillerie & Fixations", "Outillage", "Protection & Sécurité"],
    };
    const TACHES: ListType = ListType {
        label: "Tâches & Organisation",
        icon: "📝",
        questions: &["Quel est l’objectif ou le projet principal de cette liste ?", "Y a-t-il une date limite ou des priorités urgentes ?"],
        default_categories: &["Urgent", "À faire", "En attente", "Idées"],
    };
    const CADEAUX: ListType = ListType {
        label: "Cadeaux & Événements",
        icon: "🎁",
        questions: &["Pour quelle occasion (Noël, anniversaire) et pour qui sont ces cadeaux ?", "Y a-t-il un budget approximatif par personne ?"],
        default_categories: &["Idées cadeaux", "Achats prioritaires", "Emballages & Cartes"],
    };
    const CUSTOM: ListType = ListType {
        label: "Liste personnalisée",
        icon: "📋",
        questions: &["De quoi as-tu besoin exactement dans cette liste ?"],
        default_categories: &["Éléments principaux", "Divers"],
    };
    match t {
        "courses" => &COURSES,
        "voyage" => &VOYAGE,
        "bricolage" => &BRICOLAGE,
        "taches" => &TACHES,
        "cadeaux" => &CADEAUX,
        _ => &CUSTOM,
    }
}

struct Creation {
    at: i64,
    typ: String,
    topic: String,
    question_index: usize,
    answers: Vec<String>,
}

pub struct SmartLists {
    o: OrchRef,
    pub dir: PathBuf,
    log: Rc<dyn Fn(String)>,
    /// Ordre d'insertion conservé (comme la Map de la v3).
    pub lists: RefCell<Vec<J>>,
    active_creation: RefCell<Option<Creation>>,
}

fn fold(s: &str) -> String {
    js::fold_lower(s.trim())
}

fn date_ms(v: &J) -> i64 {
    js::parse_date_ms(&js::s(v)).unwrap_or(0)
}

impl SmartLists {
    pub fn new(o: OrchRef, dir: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log(format!("SmartLists mkdir error: {e}"));
        }
        let me = Self { o, dir, log, lists: RefCell::new(vec![]), active_creation: RefCell::new(None) };
        me.load_all();
        me
    }

    pub fn load_all(&self) {
        let mut out = vec![];
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(".json") || name == "index.json" {
                    continue;
                }
                if let Some(d) = js::read_json(&e.path()) {
                    if js::truthy(&d["id"]) && js::truthy(&d["title"]) {
                        out.push(d);
                    }
                }
            }
        }
        *self.lists.borrow_mut() = out;
    }

    pub fn get(&self, id: &str) -> Option<J> {
        self.lists.borrow().iter().find(|l| js::gs(l, "id") == id).cloned()
    }

    pub fn count(&self) -> usize {
        self.lists.borrow().len()
    }

    pub fn save_list(&self, list: &mut J) -> bool {
        list["updatedAt"] = json!(js::iso_now());
        let id = js::gs(list, "id");
        let file = self.dir.join(format!("{id}.json"));
        match std::fs::write(&file, js::pretty(list)) {
            Ok(_) => {
                let mut lists = self.lists.borrow_mut();
                match lists.iter_mut().find(|l| js::gs(l, "id") == id) {
                    Some(l) => *l = list.clone(),
                    None => lists.push(list.clone()),
                }
                drop(lists);
                self.sync_rag(list);
                true
            }
            Err(e) => {
                (self.log)(format!("SmartLists save error: {e}"));
                false
            }
        }
    }

    /// Liste écrite ailleurs (dépôt du site Nomade) : un nom et ses éléments, type deviné d'après le nom.
    pub fn create_from_items(&self, title: &str, items: &[String]) -> Option<J> {
        let title = js::slice_chars(title.trim(), 80);
        if title.is_empty() {
            return None;
        }
        let typ = Self::detect_type(&title);
        let now = js::iso_now();
        let items: Vec<J> = items.iter().map(|t| js::slice_chars(t.trim(), 300)).filter(|t| !t.is_empty()).take(300).enumerate()
            .map(|(i, text)| json!({ "id": i + 1, "text": text, "checked": false, "category": "" })).collect();
        let mut list = json!({ "id": format!("list_{typ}_{}", base36(js::now_ms() as u64)), "type": typ, "title": title, "context": "", "createdAt": now, "updatedAt": now, "items": items });
        self.save_list(&mut list).then_some(list)
    }

    pub fn delete_list(&self, id: &str) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        let file = self.dir.join(format!("{id}.json"));
        if file.exists() && std::fs::remove_file(&file).is_err() {
            return false;
        }
        self.lists.borrow_mut().retain(|l| js::gs(l, "id") != id);
        self.o.up().memory.forget_pinned(&format!("list_{id}"));
        true
    }

    /// Synchronise la liste avec la mémoire sémantique (épingle list_<id>).
    fn sync_rag(&self, list: &J) {
        let items = js::arr(&list["items"]);
        let remaining: Vec<String> = items.iter().filter(|i| !js::gb(i, "checked")).map(|i| js::gs(i, "text")).collect();
        let checked: Vec<String> = items.iter().filter(|i| js::gb(i, "checked")).map(|i| js::gs(i, "text")).collect();
        let mut parts = vec![format!("Liste « {} » ({}) créée le {}.", js::gs(list, "title"), js::gs(list, "type"), js::slice_chars(&js::gs(list, "createdAt"), 10))];
        if js::truthy(&list["context"]) {
            parts.push(format!("Contexte : {}.", js::gs(list, "context")));
        }
        parts.push(if remaining.is_empty() { "Tous les éléments sont terminés.".into() } else { format!("À faire / À acheter ({}) : {}.", remaining.len(), remaining.join(", ")) });
        if !checked.is_empty() {
            parts.push(format!("Terminé / Déjà acheté ({}) : {}.", checked.len(), checked.join(", ")));
        }
        self.o.up().memory.upsert_pinned(&format!("list_{}", js::gs(list, "id")), &parts.join(" "), "listes");
    }

    pub fn detect_type(text: &str) -> &'static str {
        let t = js::fold_lower(text);
        if crate::re!(r"\b(?:courses?|alimentaire|manger|repas|supermarche|epicerie|nourriture)\b").is_match(&t) {
            "courses"
        } else if crate::re!(r"\b(?:valise|voyage|partir|vacances|vol|bagage|affaires?|sejour)\b").is_match(&t) {
            "voyage"
        } else if crate::re!(r"\b(?:bricolage|travaux|outil|outillage|materiel|renovation|chantier)\b").is_match(&t) {
            "bricolage"
        } else if crate::re!(r"\b(?:cadeaux?|noel|anniversaire|fete)\b").is_match(&t) {
            "cadeaux"
        } else if crate::re!(r"\b(?:taches?|to\s*do|todo|projet|organisation|actions?|priorites?)\b").is_match(&t) {
            "taches"
        } else {
            "custom"
        }
    }

    pub fn is_creating(&self) -> bool {
        self.active_creation.borrow().is_some()
    }

    /// Création assistée en cours depuis moins de `ms` millisecondes.
    pub fn creation_fresh(&self, ms: i64) -> bool {
        self.active_creation.borrow().as_ref().map(|c| js::now_ms() - c.at < ms).unwrap_or(false)
    }

    /// Abandonne la création assistée en cours ; true s'il y en avait une.
    pub fn cancel_creation(&self) -> bool {
        self.active_creation.borrow_mut().take().is_some()
    }

    /// Démarre la création assistée d'une liste.
    pub fn start_creation(&self, raw: &str) -> J {
        let typ = Self::detect_type(raw);
        let cfg = list_type(typ);
        let clean = raw.trim().to_string();
        let immediate = clean.chars().count() > 35 || crate::re!(r"(?i)\b(?:pour\s+\d+|pour\s+ce\s+soir|demain|semaine|weekend|vacances|rome|paris)\b").is_match(&clean);
        if immediate {
            return self.generate_list(typ, &clean, &[clean.clone()]);
        }
        let q = cfg.questions[0];
        *self.active_creation.borrow_mut() = Some(Creation { at: js::now_ms(), typ: typ.into(), topic: clean, question_index: 0, answers: vec![] });
        json!({ "interactive": true, "say": q, "display": format!("Création : {}", cfg.label), "question": q })
    }

    /// Réponse de l'utilisateur pendant le raffinement.
    pub fn handle_answer(&self, answer: &str) -> Option<J> {
        let (typ, topic, answers, index) = {
            let mut slot = self.active_creation.borrow_mut();
            let c = slot.as_mut()?;
            c.answers.push(answer.trim().to_string());
            c.question_index += 1;
            c.at = js::now_ms();
            (c.typ.clone(), c.topic.clone(), c.answers.clone(), c.question_index)
        };
        let cfg = list_type(&typ);
        if crate::re!(r"(?i)^(?:annule|annuler|laisse|laisse tomber|stop|non merci)\b").is_match(answer) {
            *self.active_creation.borrow_mut() = None;
            return Some(json!({ "say": "Création de liste annulée.", "done": true }));
        }
        if crate::re!(r"(?i)^(?:c est tout|vas y|genere|directement|standard|pas d autre chose)\b").is_match(answer) || index >= cfg.questions.len() {
            *self.active_creation.borrow_mut() = None;
            return Some(self.generate_list(&typ, &topic, &answers));
        }
        let q = cfg.questions[index];
        Some(json!({ "interactive": true, "say": q, "display": format!("Raffinement liste : étape {}", index + 1), "question": q }))
    }

    pub fn generate_list(&self, typ: &str, topic: &str, answers: &[String]) -> J {
        let cfg = list_type(typ);
        let context = answers.iter().filter(|a| !a.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
        let id = format!("list_{typ}_{}", base36(js::now_ms() as u64));
        let first = answers.first().map(|a| js::slice_chars(a, 30));
        let title = match (typ, first) {
            ("courses", Some(a)) if !a.is_empty() => format!("Courses ({a})"),
            ("courses", _) => "Liste de courses".into(),
            ("voyage", Some(a)) if !a.is_empty() => format!("Valise : {a}"),
            ("voyage", _) => "Liste de voyage".into(),
            ("bricolage", Some(a)) if !a.is_empty() => format!("Bricolage : {a}"),
            ("bricolage", _) => "Liste de bricolage".into(),
            _ => {
                let t = crate::re!(r"(?i)^(?:cree|fais|prepare|nouvelle)\s+(?:moi\s+)?(?:une\s+)?liste\s+(?:de\s+|pour\s+)?").replace(topic, "");
                format!("Liste : {}", js::slice_chars(&t, 35))
            }
        };
        let now = js::iso_now();
        let mut list = json!({ "id": id, "type": typ, "title": title, "context": context, "createdAt": now, "updatedAt": now, "items": Self::build_initial_items(typ, &context) });
        let _ = cfg;
        self.save_list(&mut list);
        self.show_list_card(&list);
        let count = js::arr(&list["items"]).len();
        json!({
            "done": true, "list": list,
            "say": format!("J'ai créé ta {title} avec {count} éléments, et je l'ai mémorisée. Tu peux la cocher ou la compléter à l'écran."),
            "display": format!("{title} ({count} éléments)")
        })
    }

    fn build_initial_items(typ: &str, context: &str) -> Vec<J> {
        let c = context.to_lowercase();
        let mut items: Vec<J> = vec![];
        let mut add = |text: &str, cat: &str| {
            let id = items.len() + 1;
            items.push(json!({ "id": id, "text": text, "checked": false, "category": cat }));
        };
        match typ {
            "courses" => {
                add("Fruits & légumes frais (pommes, salade, tomates)", "Fruits & Légumes");
                add("Pain frais / Baguette", "Boulangerie");
                add("Beurre & Produits laitiers (lait, fromage)", "Frais");
                add("Féculents (pâtes, riz ou pommes de terre)", "Épicerie");
                add("Protéines (viande, poisson ou œufs)", "Frais");
                add("Café / Thé / Boissons", "Boissons");
                add("Huile d’olive, sel & poivre", "Épicerie");
                if c.contains("barbecue") || c.contains("bbq") {
                    add("Charbon de bois ou sarments", "Divers");
                    add("Merguez & Saucisses / Viande à griller", "Frais");
                    add("Sauces (moutarde, mayonnaise, barbecue)", "Épicerie");
                    add("Pain burger / Baguettes", "Boulangerie");
                }
                if c.contains("vegetarien") || c.contains("vege") {
                    add("Tofu / Légumineuses (lentilles, pois chiches)", "Épicerie");
                }
            }
            "voyage" => {
                add("Pièce d’identité / Passeport & Billets", "Papiers");
                add("Chargeur de téléphone & Adaptateur prise", "High-tech");
                add("Trousse de toilette & Brosse à dents", "Toilette");
                add("Sous-vêtements & Chaussettes (quantité adaptée)", "Vêtements");
                add("T-shirts / Hauts", "Vêtements");
                add("Pantalons / Shorts", "Vêtements");
                add("Chaussures de marche confortables", "Vêtements");
                add("Petite trousse de secours & Médicaments de base", "Santé");
                if c.contains("froid") || c.contains("hiver") || c.contains("ski") {
                    add("Manteau chaud, gants & bonnet", "Vêtements");
                }
                if c.contains("chaud") || c.contains("soleil") || c.contains("plage") {
                    add("Maillot de bain & Crème solaire", "Pratique");
                    add("Lunettes de soleil", "Pratique");
                }
            }
            "bricolage" => {
                add("Mètre ruban & Crayon de menuisier", "Outillage");
                add("Niveau à bulle", "Outillage");
                add("Gants de protection & Lunettes de sécurité", "Protection");
                add("Chevilles & Vis adaptées au support", "Quincaillerie");
                add("Visseuse / Perceuse chargée", "Outillage");
            }
            _ => {
                add("Définir les étapes clés", "Organisation");
                add("Rassembler les informations nécessaires", "Préparation");
                add("Planifier les échéances", "Action");
            }
        }
        items
    }

    /// La liste la plus pertinente (par type ou titre), sinon la dernière.
    pub fn find_list(&self, type_or_title: &str) -> Option<J> {
        let key = fold(type_or_title);
        let lists = self.lists.borrow();
        if key.is_empty() {
            let mut sorted: Vec<&J> = lists.iter().collect();
            sorted.sort_by_key(|l| std::cmp::Reverse(date_ms(&l["updatedAt"])));
            return sorted.first().map(|l| (*l).clone());
        }
        if let Some(l) = lists.iter().find(|l| js::gs(l, "type") == key) {
            return Some(l.clone());
        }
        if let Some(l) = lists.iter().find(|l| fold(&js::gs(l, "title")).contains(&key) || key.contains(&js::gs(l, "type"))) {
            return Some(l.clone());
        }
        // Aucun nom ne correspond : pas de liste au hasard (« ajoute X à ma liste vinyle » ne doit pas finir dans les courses).
        None
    }

    pub fn show_list_card(&self, list: &J) -> J {
        let items = js::arr(&list["items"]);
        let total = items.len();
        let checked = items.iter().filter(|i| js::gb(i, "checked")).count();
        let cfg = list_type(&js::gs(list, "type"));
        let lid = js::gs(list, "id");
        let rows: Vec<J> = items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let on = js::gb(item, "checked");
                let target = format!("{lid}:{}", js::s(&item["id"]));
                let sub = if js::truthy(&item["category"]) { js::gs(item, "category") } else if on { "Terminé".into() } else { "À faire".into() };
                let mut row = json!({
                    "icon": if on { "☑" } else { "☐" },
                    "title": format!("{}. {}", idx + 1, js::gs(item, "text")),
                    "sub": sub,
                    "call": { "method": "listAction", "args": ["smartlist", target, "toggle"] },
                    "hint": if on { "Clic : marquer à faire" } else { "Clic : marquer terminé" },
                    "actions": [
                        { "icon": if on { "↺" } else { "✓" }, "title": if on { "Décocher" } else { "Cocher" }, "call": { "method": "listAction", "args": ["smartlist", target, "toggle"] } },
                        { "icon": "✕", "tone": "danger", "title": "Supprimer cet élément", "call": { "method": "listAction", "args": ["smartlist", target, "delete_item"] } }
                    ]
                });
                if on {
                    row["tone"] = json!("ok");
                }
                row
            })
            .collect();
        let card = json!({
            "title": format!("{} {}", cfg.icon, js::gs(list, "title")),
            "tool": "liste intelligente", "wide": true, "count": format!("{checked}/{total}"), "ttl": 300000,
            "rows": rows, "empty": "Cette liste est vide.",
            "footer": "« coche le 2 » · « ajoute [élément] » · 📋 copier la liste · ✕ supprimer"
        });
        self.o.up().show_widget("tool", card.clone());
        card
    }

    /// Actions depuis les boutons de card ou les requêtes vocales.
    pub async fn action(&self, target: &str, act: &str, params: &J) -> J {
        let mut parts = target.splitn(2, ':');
        let list_id = parts.next().unwrap_or("").to_string();
        let item_id: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
        let Some(mut list) = self.get(&list_id).or_else(|| self.find_list(&list_id)) else { return json!({ "ok": false, "say": "Liste introuvable." }) };
        let same = |i: &J| js::num(&i["id"]) == Some(item_id);
        match act {
            "toggle" => {
                let mut said = None;
                if let Some(items) = list["items"].as_array_mut() {
                    if let Some(item) = items.iter_mut().find(|i| same(i)) {
                        let on = !js::gb(item, "checked");
                        item["checked"] = json!(on);
                        said = Some(format!("{} : {}.", if on { "Coché" } else { "Décoché" }, js::gs(item, "text")));
                    }
                }
                if let Some(say) = said {
                    self.save_list(&mut list);
                    self.show_list_card(&list);
                    return json!({ "ok": true, "say": say });
                }
            }
            "delete_item" => {
                let mut removed = None;
                if let Some(items) = list["items"].as_array_mut() {
                    if let Some(idx) = items.iter().position(|i| same(i)) {
                        removed = Some(items.remove(idx));
                    }
                }
                if let Some(r) = removed {
                    self.save_list(&mut list);
                    self.show_list_card(&list);
                    return json!({ "ok": true, "say": format!("Retiré : {}.", js::gs(&r, "text")) });
                }
            }
            "add_item" => {
                let text = js::gs(params, "text").trim().to_string();
                if text.is_empty() {
                    return json!({ "ok": false, "say": "Texte d’élément manquant." });
                }
                let cat = if js::truthy(&params["category"]) { js::gs(params, "category") } else { "Divers".into() };
                if let Some(items) = list["items"].as_array_mut() {
                    let id = items.len() + 1;
                    items.push(json!({ "id": id, "text": text, "checked": false, "category": cat }));
                }
                self.save_list(&mut list);
                self.show_list_card(&list);
                return json!({ "ok": true, "say": format!("Ajouté à la liste : {text}.") });
            }
            "copy" | "copier" => {
                let text = Self::export_text(&list);
                let _ = crate::orch::system::presse_papier_ecrire(&json!({ "texte": text })).await;
                return json!({ "ok": true, "say": format!("La liste « {} » a été copiée dans le presse-papier.", js::gs(&list, "title")) });
            }
            "delete" | "supprimer" => {
                self.delete_list(&js::gs(&list, "id"));
                return json!({ "ok": true, "say": format!("Liste « {} » supprimée.", js::gs(&list, "title")) });
            }
            "show" => {
                self.show_list_card(&list);
                return json!({ "ok": true, "say": js::gs(&list, "title") });
            }
            _ => {}
        }
        json!({ "ok": false, "say": "Action non reconnue." })
    }

    pub fn export_text(list: &J) -> String {
        let mut lines = vec![format!("# {}", js::gs(list, "title")), if js::truthy(&list["context"]) { format!("> {}\n", js::gs(list, "context")) } else { String::new() }];
        for item in js::arr(&list["items"]) {
            lines.push(format!("- [{}] {}", if js::gb(&item, "checked") { "x" } else { " " }, js::gs(&item, "text")));
        }
        lines.join("\n")
    }

    pub fn show_overview_card(&self) -> J {
        let mut all = self.lists.borrow().clone();
        all.sort_by_key(|l| std::cmp::Reverse(date_ms(&l["updatedAt"])));
        let rows: Vec<J> = all
            .iter()
            .enumerate()
            .map(|(idx, list)| {
                let items = js::arr(&list["items"]);
                let checked = items.iter().filter(|i| js::gb(i, "checked")).count();
                let cfg = list_type(&js::gs(list, "type"));
                let id = js::gs(list, "id");
                let date = js::local_of(date_ms(&list["updatedAt"])).format("%d/%m/%Y").to_string();
                json!({
                    "icon": cfg.icon,
                    "title": format!("{}. {}", idx + 1, js::gs(list, "title")),
                    "sub": format!("{checked}/{} cochés · {date}", items.len()),
                    "call": { "method": "listAction", "args": ["smartlist", id, "show"] },
                    "hint": "Clic : ouvrir cette liste",
                    "actions": [
                        { "icon": "👁️", "title": "Ouvrir", "call": { "method": "listAction", "args": ["smartlist", id, "show"] } },
                        { "icon": "📋", "title": "Copier", "call": { "method": "listAction", "args": ["smartlist", id, "copy"] } },
                        { "icon": "✕", "tone": "danger", "title": "Supprimer", "call": { "method": "listAction", "args": ["smartlist", id, "delete"] } }
                    ]
                })
            })
            .collect();
        let card = json!({
            "title": "Mes listes", "tool": "smartlists", "count": all.len(), "ttl": 120000, "rows": rows,
            "empty": "Aucune liste enregistrée. Dis par exemple : « Crée-moi une liste de courses ».",
            "footer": "« ouvre ma liste de courses » · « crée une liste de voyage » · 📋 copier"
        });
        self.o.up().show_widget("tool", card.clone());
        card
    }
}

fn not_found(target: &str) -> J {
    json!({ "say": format!("Je ne trouve pas de liste correspondant à « {target} »."), "display": "Liste introuvable" })
}

/// Intentions des listes intelligentes (texte replié).
pub fn match_smart_list(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    let raw = if raw.is_empty() { t.to_string() } else { raw.to_string() };
    let oc = o.clone();
    if crate::re!(r"^(?:quelles sont mes listes|liste (?:mes|toutes mes|les) listes|mes listes|toutes mes listes|affiche mes listes)$").is_match(t) {
        return Some(Intent::new("smartlists_overview", "Mes listes", "récapitulatif", move || {
            let o = oc.clone();
            async move {
                let orch = o.up();
                orch.smart_lists.show_overview_card();
                let n = orch.smart_lists.count();
                let s = if n > 1 { "s" } else { "" };
                json!({ "say": if n > 0 { format!("Tu as {n} liste{s} enregistrée{s}.") } else { "Aucune liste enregistrée pour l’instant.".into() }, "display": format!("{n} listes") })
            }
        }));
    }
    let other = crate::re!(r"\b(?:notes?|reunions?|visios?|projets?|mails?|emails?|rappels?|applications?|applis?|fenetres?|fichiers?|automatisations?|raccourcis?|commandes?|gmeet)\b").is_match(t);
    if other {
        return None;
    }
    if let Some(m) = crate::re!(r"(?i)^(?:(?:cree|creer|fais|fait|prepare|preparer|nouvelle)(?: moi)? (?:une )?liste (?:de |d |pour )?(.+)|liste (?:de |d )?(?:courses?|voyage|valise|bricolage|cadeaux?|taches?))$").captures(t) {
        let topic = m.get(1).map(|x| x.as_str().to_string()).unwrap_or_else(|| t.to_string());
        return Some(Intent::new("smartlist_create", "Créer une liste", &topic, move || {
            let (o, raw) = (oc.clone(), raw.clone());
            async move {
                let res = o.up().smart_lists.start_creation(&raw);
                json!({ "say": res["say"], "display": if js::truthy(&res["display"]) { res["display"].clone() } else { json!("Liste intelligente") } })
            }
        }).free_text());
    }
    if let Some(m) = crate::re!(r"(?i)^(?:affiche|montre|ouvre|donne|consulte|lis)(?: moi)? (?:la |ma )?liste (?:de |d |pour )?(.+)$").captures(t) {
        let target = m[1].trim().to_string();
        return Some(Intent::new("smartlist_show", "Ouvrir une liste", &target.clone(), move || {
            let (o, target) = (oc.clone(), target.clone());
            async move {
                let orch = o.up();
                let Some(list) = orch.smart_lists.find_list(&target) else { return not_found(&target) };
                orch.smart_lists.show_list_card(&list);
                let remaining = js::arr(&list["items"]).iter().filter(|i| !js::gb(i, "checked")).count();
                json!({ "say": format!("Voici ta {}. Il reste {remaining} élément{} à faire.", js::gs(&list, "title"), if remaining > 1 { "s" } else { "" }), "display": list["title"] })
            }
        }));
    }
    if let Some(m) = crate::re!(r"(?i)^(?:copie|copier)(?: moi)? (?:la |ma )?liste (?:de |d |pour )?(.+)$").captures(t) {
        let target = m[1].trim().to_string();
        return Some(Intent::new("smartlist_copy", "Copier la liste", &target.clone(), move || {
            let (o, target) = (oc.clone(), target.clone());
            async move {
                let orch = o.up();
                let Some(list) = orch.smart_lists.find_list(&target) else { return not_found(&target) };
                let res = orch.smart_lists.action(&js::gs(&list, "id"), "copy", &json!({})).await;
                json!({ "say": res["say"], "display": "Liste copiée" })
            }
        }));
    }
    if let Some(m) = crate::re!(r"(?i)^(?:ajoute|rajoute|mets?)(?: moi)? (.+?) (?:dans|a|sur) (?:la |ma )?liste (?:de |d |pour )?(.+)$").captures(t) {
        let item = m[1].trim().to_string();
        let target = m[2].trim().to_string();
        return Some(Intent::new("smartlist_add_item", "Ajouter à la liste", &format!("{item} → {target}"), move || {
            let (o, item, target) = (oc.clone(), item.clone(), target.clone());
            async move {
                let orch = o.up();
                let Some(list) = orch.smart_lists.find_list(&target) else { return not_found(&target) };
                let res = orch.smart_lists.action(&js::gs(&list, "id"), "add_item", &json!({ "text": item })).await;
                json!({ "say": res["say"], "display": format!("+ {item}") })
            }
        }).free_text());
    }
    if let Some(m) = crate::re!(r"(?i)^(?:coche|marque comme fait|valide)(?: moi)? (?:l'element |le |la )?(.+?) (?:dans|sur) (?:la |ma )?liste(?: (?:de |d |pour )?(.+))?$").captures(t) {
        let item_target = m[1].trim().to_string();
        let target = m.get(2).map(|x| x.as_str().trim().to_string()).unwrap_or_default();
        return Some(Intent::new("smartlist_check_item", "Cocher élément", &item_target.clone(), move || {
            let (o, item_target, target) = (oc.clone(), item_target.clone(), target.clone());
            async move {
                let orch = o.up();
                let sl = &orch.smart_lists;
                let Some(mut list) = sl.find_list(&target) else { return json!({ "say": "Je ne trouve pas cette liste.", "display": "Liste introuvable" }) };
                let items = list["items"].as_array_mut();
                let Some(items) = items else { return json!({ "say": format!("Je ne trouve pas « {item_target} » dans la liste."), "display": "Élément introuvable" }) };
                let idx = if item_target.bytes().all(|b| b.is_ascii_digit()) {
                    item_target.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).filter(|i| *i < items.len())
                } else {
                    let norm = fold(&item_target);
                    items.iter().position(|i| fold(&js::gs(i, "text")).contains(&norm))
                };
                let Some(idx) = idx else { return json!({ "say": format!("Je ne trouve pas « {item_target} » dans la liste."), "display": "Élément introuvable" }) };
                items[idx]["checked"] = json!(true);
                let text = js::gs(&items[idx], "text");
                sl.save_list(&mut list);
                sl.show_list_card(&list);
                json!({ "say": format!("C'est fait : {text} est coché."), "display": format!("✓ {text}") })
            }
        }));
    }
    if let Some(m) = crate::re!(r"(?i)^(?:supprime|efface|retire)(?: moi)? (?:la |ma )?liste (?:de |d |pour )?(.+)$").captures(t) {
        let target = m[1].trim().to_string();
        return Some(Intent::new("smartlist_delete", "Supprimer liste", &target.clone(), move || {
            let (o, target) = (oc.clone(), target.clone());
            async move {
                let orch = o.up();
                let Some(list) = orch.smart_lists.find_list(&target) else { return not_found(&target) };
                let res = orch.smart_lists.action(&js::gs(&list, "id"), "delete", &json!({})).await;
                json!({ "say": res["say"], "display": "Liste supprimée" })
            }
        }));
    }
    None
}

#[allow(unused)]
fn _v(_: Value) {}
