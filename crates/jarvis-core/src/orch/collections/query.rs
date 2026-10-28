// Interroger ses listes : « est-ce que j'ai le Lego 75436 ? », « affiche-moi Thriller », « combien j'ai de vinyles »,
// « combien j'ai dépensé en Lego », « quels disques de U2 j'ai », « j'ai acheté Thriller 25 euros à la Fnac »,
// « écoute le vinyle Thriller », « donne-moi le prix du Lego 75436 », « ajoute un champ pays à ma liste vinyle ».
// Trouver une fiche est déterministe d'abord (identifiant, titre) : aucune IA, aucun jeton. Le RAG (sens, mots)
// ne sert qu'ensuite. Les mêmes recherches sont offertes à l'IA par les outils listes_chercher et listes_lire.

use serde_json::{json, Value};

use super::super::intent::Intent;
use super::super::shopping;
use super::super::{OrchRef, Up};
use super::schema::{self, fold, Statut};
use super::service::Collections;
use super::voice::{parse_item_text, Conversation, Stage};
use crate::js::{self, J};

fn say(text: impl Into<String>, display: impl Into<String>) -> J {
    json!({ "say": text.into(), "display": display.into() })
}

fn trim_end_punct(s: &str) -> String {
    crate::re!(r"[\s.!?…]+$").replace(s.trim(), "").to_string()
}

fn strip_det(s: &str) -> String {
    crate::re!(r"(?i)^(?:(?:le|la|les|un|une|des|du|mon|ma|mes|ce|cette|ces)\s+|l['’]|d['’])").replace(s.trim(), "").to_string()
}

fn euros(n: f64) -> String {
    shopping::fmt_price(n)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n > 1 { many } else { one })
}

/// Ce que tu sais de l'achat d'une fiche : « Acheté 89 € chez Fnac le 12/03/2024. »
/// La clé du champ « prix » d'une liste : celui de la pastille, sinon « prix payé ».
pub fn price_key(list: &J) -> Option<String> {
    js::str_list(&list["vignette"]).into_iter().find(|k| schema::field(list, k).map(|f| js::gs(f, "type") == "prix").unwrap_or(false)).or_else(|| schema::field(list, "prix_achat").map(|_| "prix_achat".to_string()))
}

pub fn purchase_phrase(list: &J, item: &J) -> String {
    // Film, série, concert : ce qui compte, c'est quand, où et ta note.
    if js::gs(&schema::vocab(list), "fait") != "Possédé" {
        let g = |k: &str| schema::display(list, item, k);
        let mut parts: Vec<String> = vec![];
        for (k, label) in [("date_vue", "Vu le"), ("date", "Le"), ("salle", "à"), ("ville", "à"), ("avancement", "Où tu en es :"), ("note", "Ta note :")] {
            let v = g(k);
            if !v.is_empty() {
                parts.push(if k == "note" { format!("{label} {v} sur 10") } else { format!("{label} {v}") });
            }
        }
        if let Some(k) = price_key(list) {
            let p = g(&k);
            if !p.is_empty() {
                parts.push(format!("billet {p}"));
            }
        }
        return if parts.is_empty() { String::new() } else { format!("{}.", parts.join(", ")) };
    }
    let price = price_key(list).map(|k| schema::display(list, item, &k)).unwrap_or_default();
    let place = ["lieu_achat", "magasin"].iter().map(|k| schema::display(list, item, k)).find(|s| !s.is_empty()).unwrap_or_default();
    let date = schema::display(list, item, "date_achat");
    if price.is_empty() && place.is_empty() && date.is_empty() {
        return String::new();
    }
    let mut s = String::from("Acheté");
    if !price.is_empty() {
        s.push_str(&format!(" {price}"));
    }
    if !place.is_empty() {
        let low = place.to_lowercase();
        let article = ["la ", "le ", "les ", "l'", "l’"].iter().any(|a| low.starts_with(a));
        s.push_str(&format!(" {}{place}", if article { "à " } else { "chez " }));
    }
    if !date.is_empty() {
        s.push_str(&format!(" le {date}"));
    }
    s.push('.');
    s
}

/// (total dépensé, fiches possédées avec un prix, fiches possédées).
pub fn spent(list: &J) -> (f64, usize, usize) {
    let mut total = 0.0;
    let (mut priced, mut owned) = (0, 0);
    for it in js::arr(&list["elements"]) {
        if schema::statut_of(list, &it) == Statut::Souhaite {
            continue;
        }
        owned += 1;
        if let Some(p) = price_key(list).and_then(|k| js::num(schema::value(&it, &k))) {
            total += p;
            priced += 1;
        }
    }
    (js::round2(total), priced, owned)
}

/// Statistiques d'une liste : totaux, dépenses et répartitions par champ (texte court, choix, année d'une date) ;
/// seuls les champs où des valeurs se répètent sont retenus (un champ tout différent n'apprend rien).
pub fn statistics(list: &J) -> J {
    let items = js::arr(&list["elements"]);
    let (spent_total, priced, owned) = spent(list);
    let wished = items.iter().filter(|i| schema::statut_of(list, i) == Statut::Souhaite).count();
    let skip: Vec<String> = [vec![js::gs(list, "titre"), js::gs(list, "etat")], js::str_list(&list["identifiant"])].concat();
    let mut groups: Vec<J> = vec![];
    for f in js::arr(&list["champs"]) {
        let (key, typ) = (js::gs(&f, "cle"), js::gs(&f, "type"));
        if skip.contains(&key) || !["texte", "choix", "date"].contains(&typ.as_str()) {
            continue;
        }
        let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for it in &items {
            let v = schema::value(it, &key);
            let shown = if typ == "date" { js::s(v).chars().take(4).collect::<String>() } else { schema::display(list, it, &key) };
            if !shown.trim().is_empty() {
                *counts.entry(shown).or_insert(0) += 1;
            }
        }
        let filled: usize = counts.values().sum();
        if filled < 2 || (typ != "choix" && counts.len() == filled) {
            continue;
        }
        let mut rows: Vec<(String, usize)> = counts.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        rows.truncate(8);
        let label = if typ == "date" { format!("{} (par année)", js::gs(&f, "libelle")) } else { js::gs(&f, "libelle") };
        groups.push(json!({ "cle": key, "libelle": label, "lignes": rows.iter().map(|(v, n)| json!({ "valeur": v, "nombre": n })).collect::<Vec<_>>() }));
    }
    json!({
        "liste": list["nom"], "total": items.len(), "possedes": owned, "souhaites": wished,
        "depense": spent_total, "avecPrix": priced, "repartitions": groups
    })
}

impl Collections {
    /// Fiches d'une liste qui correspondent à un texte : identifiant, titre exact, titre contenu (ou le contenant).
    pub fn find_item(list: &J, text: &str) -> Vec<J> {
        let q = fold(&strip_leading_noun_of(list, text));
        let Some(items) = list["elements"].as_array() else { return vec![] };
        if q.is_empty() {
            return vec![];
        }
        let words: Vec<&str> = q.split(' ').collect();
        let by_id: Vec<&J> = items.iter().filter(|i| schema::identifiers_of(list, i).iter().any(|id| words.contains(&fold(id).as_str()))).collect();
        if !by_id.is_empty() {
            return by_id.into_iter().cloned().collect();
        }
        let titles: Vec<String> = items.iter().map(|i| fold(&schema::title_of(list, i))).collect();
        let exact: Vec<J> = items.iter().zip(&titles).filter(|(_, t)| **t == q).map(|(i, _)| i.clone()).collect();
        if !exact.is_empty() {
            return exact;
        }
        let padded = format!(" {q} ");
        items.iter().zip(&titles).filter(|(_, t)| t.contains(&q) || (t.len() >= 5 && padded.contains(&format!(" {t} ")))).map(|(i, _)| i.clone()).collect()
    }

    /// Fiches de toutes les listes (ou d'une seule) qui correspondent au texte. Les listes ne sont clonées que si elles ont un résultat.
    pub fn quick_find(&self, text: &str, only: Option<&J>) -> Vec<(J, J)> {
        let mut out = vec![];
        for l in self.lists.borrow().iter() {
            if only.map(|o| js::gs(o, "id") != js::gs(l, "id")).unwrap_or(false) {
                continue;
            }
            for i in Self::find_item(l, text) {
                out.push((l.clone(), i));
            }
        }
        out
    }

    /// La liste dont un nom ou un alias apparaît (mot entier) dans la phrase.
    pub fn list_in_text(&self, text: &str) -> Option<J> {
        let padded = format!(" {} ", super::service::loose(text));
        let mut best: Option<(usize, J)> = None;
        for l in self.lists.borrow().iter() {
            for a in js::str_list(&l["alias"]).into_iter().chain([js::gs(l, "nom")]) {
                let n = super::service::loose(&a);
                if !n.is_empty() && padded.contains(&format!(" {n} ")) && best.as_ref().map(|(b, _)| n.len() > *b).unwrap_or(true) {
                    best = Some((n.len(), l.clone()));
                }
            }
        }
        best.map(|(_, l)| l)
    }

    // --- Pour l'IA (outils listes_chercher et listes_lire) ----------------------------------------

    pub async fn ai_search_text(&self, query: &str, list_phrase: &str, limit: usize) -> String {
        let only = if list_phrase.trim().is_empty() { None } else { self.find_by_phrase(list_phrase) };
        let mut lines: Vec<String> = vec![];
        let mut seen: Vec<String> = vec![];
        for (l, i) in self.quick_find(query, only.as_ref()) {
            seen.push(js::gs(&i, "id"));
            lines.push(Self::ai_line(&l, &i, "trouvée par son titre ou son numéro"));
        }
        for (l, i, h) in self.search(query, only.as_ref().map(|l| js::gs(l, "id")).as_deref(), limit).await {
            if !seen.contains(&js::gs(&i, "id")) {
                lines.push(Self::ai_line(&l, &i, &format!("{} (score {:.2})", if h.exact { "identifiant exact" } else { "par le sens" }, h.score)));
            }
        }
        if lines.is_empty() {
            return "Aucune fiche ne correspond dans les listes de l'utilisateur.".into();
        }
        lines.truncate(limit.max(1));
        lines.join("\n")
    }

    fn ai_line(list: &J, item: &J, how: &str) -> String {
        let status = match schema::statut_of(list, item) {
            Statut::Possede => "possédé",
            Statut::Souhaite => "souhaité (pas encore possédé)",
            Statut::Inconnu => "",
        };
        let mut s = format!("- [{}] {} — id {}/{} — {status}", js::gs(list, "nom"), schema::passage(list, item), js::gs(list, "id"), js::gs(item, "id"));
        s.push_str(&format!(" ({how})"));
        s
    }

    pub fn ai_read_text(&self, list_phrase: &str, item_phrase: &str) -> String {
        if list_phrase.trim().is_empty() {
            let lists = self.all();
            if lists.is_empty() {
                return "L'utilisateur n'a aucune liste à fiches.".into();
            }
            return lists.iter().map(|l| format!("- {} (alias : {}) : {}", js::gs(l, "nom"), js::str_list(&l["alias"]).join(", "), schema::summary(l))).collect::<Vec<_>>().join("\n");
        }
        let Some(list) = self.find_by_phrase(list_phrase) else { return format!("Aucune liste ne s'appelle « {list_phrase} ».") };
        if !item_phrase.trim().is_empty() {
            let found = Self::find_item(&list, item_phrase);
            return match found.len() {
                0 => format!("Aucune fiche « {item_phrase} » dans la liste {}.", js::gs(&list, "nom")),
                _ => found.iter().take(5).map(|i| schema::passage(&list, i)).collect::<Vec<_>>().join("\n"),
            };
        }
        let (total, priced, owned) = spent(&list);
        let mut out = vec![format!("{} — {} dépensés sur {} possédés ({} avec un prix noté).", schema::summary(&list), euros(total), owned, priced)];
        let items = js::arr(&list["elements"]);
        for i in items.iter().take(300) {
            out.push(format!("- {} [{}]{}", schema::title_of(&list, i), schema::statut_of(&list, i).code(), { let s = schema::subtitle_of(&list, i); if s.is_empty() { s } else { format!(" — {s}") } }));
        }
        if items.len() > 300 {
            out.push(format!("… et {} autres (précise un titre pour la fiche complète).", items.len() - 300));
        }
        out.join("\n")
    }

    // --- Offre d'ajout après un « non » -------------------------------------------------------------

    pub(super) fn offer_add(&self, list: &J, text: &str) {
        *self.conv.borrow_mut() = Some(Conversation { at: js::now_ms(), stage: Stage::OfferAdd { list: js::gs(list, "id"), text: text.to_string() } });
    }
}

fn strip_leading_noun_of(list: &J, text: &str) -> String {
    let t = strip_det(text);
    let mut nouns: Vec<String> = js::str_list(&list["alias"]);
    nouns.extend([js::gs(&list["noms"], "singulier"), js::gs(&list["noms"], "pluriel")]);
    nouns.retain(|n| !n.trim().is_empty());
    nouns.sort_by_key(|n| std::cmp::Reverse(n.len()));
    let alt = nouns.iter().map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
    if alt.is_empty() {
        return t;
    }
    match regex::Regex::new(&format!(r"(?i)^(?:{alt})\s+(?:de\s+|du\s+|d['’]\s*)?")) {
        Ok(re) => {
            let s = re.replace(&t, "").to_string();
            if s.trim().is_empty() { t } else { s }
        }
        Err(_) => t,
    }
}

const ASK: &str = r"(?:est[- ]ce que\s+)?(?:j['’]ai|je poss[èe]de|ai[- ]je|on a|je n['’]ai pas|j['’]ai pas)";

/// Intentions de recherche et d'action sur les fiches. `raw` : phrase d'origine sans ponctuation finale.
pub fn match_query(o: &OrchRef, cols: &Collections, raw: &str, late: bool) -> Option<Intent> {
    let oc = o.clone();
    if cols.count() == 0 {
        return None;
    }
    if late {
        return match_open_item(oc, cols, raw);
    }

    // Champ d'une liste à la voix (avant l'ajout de fiche, qui reconnaîtrait « ajoute … à ma liste »).
    if let Some(m) = crate::re!(r"(?i)^(?:ajoute|rajoute)(?:[- ]moi)?\s+(?:un|le|une|la)\s+(?:nouveau\s+|nouvelle\s+)?(?:champ|colonne)\s+(.+?)\s+(?:[àa]|dans|sur)\s+(?:ma|la|mes)\s+(?:liste|collection)s?(?:\s+(?:de|d['’]|des|du)\s*|\s+)(.+)$").captures(raw) {
        if let Some(list) = cols.find_by_phrase(&m[2]) {
            let label = js::capitalize(m[1].trim());
            let typ = super::voice::guess_type(&label);
            return Some(Intent::new("collection_add_field", "Ajouter un champ", &label.clone(), move || {
                let (o, id, label) = (oc.clone(), js::gs(&list, "id"), label.clone());
                async move {
                    match o.up().collections.update_schema(&id, &json!({ "ajouter": [{ "libelle": label, "type": typ }] }), "voix") {
                        Ok(l) => say(format!("J'ai ajouté le champ « {label} » à ta liste {}. Dis « annule » pour le retirer.", js::gs(&l, "nom")), format!("+ champ {label}")),
                        Err(e) => say(e, "Erreur"),
                    }
                }
            }).free_text());
        }
    }
    if let Some(m) = crate::re!(r"(?i)^(?:retire|supprime|enl[èe]ve|efface)(?:[- ]moi)?\s+(?:le|la)\s+(?:champ|colonne)\s+(.+?)\s+(?:de|dans|sur)\s+(?:ma|la|mes)\s+(?:liste|collection)s?(?:\s+(?:de|d['’]|des|du)\s*|\s+)(.+)$").captures(raw) {
        if let Some(list) = cols.find_by_phrase(&m[2]) {
            let wanted = fold(&m[1]);
            if let Some(f) = js::arr(&list["champs"]).into_iter().find(|f| fold(&js::gs(f, "libelle")) == wanted || js::gs(f, "cle") == wanted.replace(' ', "_")) {
                let key = js::gs(&f, "cle");
                return Some(Intent::new("collection_remove_field", "Retirer un champ", &key.clone(), move || {
                    let (o, id, key) = (oc.clone(), js::gs(&list, "id"), key.clone());
                    async move {
                        match o.up().collections.update_schema(&id, &json!({ "retirer": [key] }), "voix") {
                            Ok(l) => say(format!("Champ retiré de ta liste {}. Dis « annule » pour le remettre.", js::gs(&l, "nom")), "Champ retiré"),
                            Err(e) => say(e, "Erreur"),
                        }
                    }
                }));
            }
        }
    }
    if let Some(m) = crate::re!(r"(?i)^renomme(?:[- ]moi)?\s+(?:ma|la)\s+(?:liste|collection)\s+(.+?)\s+en\s+(.+)$").captures(raw) {
        if let Some(list) = cols.find_by_phrase(&m[1]) {
            let name = schema::clean_text(m[2].trim().trim_matches(|c| c == '«' || c == '»' || c == '"'), 60);
            return Some(Intent::new("collection_rename", "Renommer une liste", &name.clone(), move || {
                let (o, id, name) = (oc.clone(), js::gs(&list, "id"), name.clone());
                async move {
                    match o.up().collections.update_schema(&id, &json!({ "nom": name }), "voix") {
                        Ok(l) => say(format!("Ta liste s'appelle maintenant « {} ».", js::gs(&l, "nom")), "Liste renommée"),
                        Err(e) => say(e, "Erreur"),
                    }
                }
            }).free_text());
        }
    }

    // Combien ai-je dépensé ? (avant « combien en ai-je ? »)
    if let Some(m) = crate::re!(r"(?i)^combien\s+(?:j['’]ai|ai[- ]je|est[- ]ce que j['’]ai|ça m['’]a|ca m['’]a)\s+(?:d[ée]pens[ée]|pay[ée]|investi|co[uû]t[ée])\s+(?:en|pour|sur|dans|mes?|mon|ma)?\s*(.+)$").captures(raw) {
        if let Some(list) = cols.list_in_text(&m[1]).or_else(|| cols.find_by_phrase(&m[1])) {
            return Some(Intent::new("collection_spent", "Dépenses", &js::gs(&list, "nom"), move || {
                let (o, id) = (oc.clone(), js::gs(&list, "id"));
                async move {
                    let Some(l) = o.up().collections.get(&id) else { return say("Cette liste n'existe plus.", "Introuvable") };
                    let (total, priced, owned) = spent(&l);
                    if priced == 0 {
                        return say(format!("Je n'ai aucun prix noté dans ta liste {}.", js::gs(&l, "nom")), "Aucun prix");
                    }
                    say(
                        format!("Tu as dépensé {} en {} : {} avec un prix noté sur {owned} possédés.", shopping::spoken_price(total), js::gs(&l, "nom"), priced),
                        euros(total),
                    )
                }
            }));
        }
    }

    // Combien en ai-je ? / Combien ai-je dépensé ?
    let count_re = crate::re!(&format!(r"(?i)^combien\s+(?:de\s+|d['’])?(.+?)\s+{ASK}(?:\s+vu[es]?)?(?:\s+en\s+tout)?$"));
    let count_re2 = crate::re!(&format!(r"(?i)^combien\s+{ASK}\s+(?:de\s+|d['’])?(.+?)(?:\s+en\s+tout)?$"));
    if let Some(m) = count_re.captures(raw).or_else(|| count_re2.captures(raw)) {
        if let Some(list) = cols.list_in_text(&m[1]) {
            return Some(Intent::new("collection_count", "Combien", &js::gs(&list, "nom"), move || {
                let (o, id) = (oc.clone(), js::gs(&list, "id"));
                async move {
                    let Some(l) = o.up().collections.get(&id) else { return say("Cette liste n'existe plus.", "Introuvable") };
                    let (total, owned, wish) = schema::counts(&l);
                    let noun = js::gs(&l["noms"], "pluriel");
                    say(
                        if wish > 0 || owned > 0 { let voc = schema::vocab(&l); let voulu = js::gs(&voc, "voulu"); format!("{} {owned} {noun}{}.", js::gs(&voc, "compteFait"), if wish > 0 { format!(", et {wish} {}", if voulu == "Souhaité" { "dans tes souhaits".to_string() } else { voulu.to_lowercase() }) } else { String::new() }) } else { format!("Ta liste {} contient {total} {noun}.", js::gs(&l, "nom")) },
                        format!("{owned} possédés"),
                    )
                }
            }));
        }
    }
    // Quels … de … ai-je ?
    if let Some(m) = crate::re!(&format!(r"(?i)^(?:quels|quelles|lesquels|lesquelles)\s+(.+?)\s+(?:de|du|d['’]|par|avec)\s+(.+?)\s+{ASK}$")).captures(raw) {
        if let Some(list) = cols.list_in_text(&m[1]) {
            let filter = strip_det(&m[2]);
            let owned_only = !crate::re!(r"(?i)\b(?:je n['’]ai pas|j['’]ai pas)\b").is_match(raw);
            return Some(Intent::new("collection_which", "Lesquels", &filter.clone(), move || {
                let (o, id, filter) = (oc.clone(), js::gs(&list, "id"), filter.clone());
                async move {
                    let Some(l) = o.up().collections.get(&id) else { return say("Cette liste n'existe plus.", "Introuvable") };
                    let q = fold(&filter);
                    let hits: Vec<J> = js::arr(&l["elements"]).into_iter().filter(|i| {
                        let own = schema::statut_of(&l, i) != Statut::Souhaite;
                        (own == owned_only) && fold(&schema::public_item(&l, i)["affichage"].as_object().map(|o| o.values().map(js::s).collect::<Vec<_>>().join(" ")).unwrap_or_default()).contains(&q)
                    }).collect();
                    if hits.is_empty() {
                        return say(format!("Je ne trouve rien pour « {filter} » dans ta liste {}.", js::gs(&l, "nom")), "Aucun résultat");
                    }
                    let names: Vec<String> = hits.iter().take(6).map(|i| schema::title_of(&l, i)).collect();
                    say(format!("{} : {}{}.", plural(hits.len(), "résultat", "résultats"), names.join(", "), if hits.len() > 6 { " et d'autres" } else { "" }), format!("{} fiches", hits.len()))
                }
            }).free_text());
        }
    }

    // J'ai acheté … / passe … en possédé.
    if let Some(m) = crate::re!(r"(?i)^(?:j['’]ai\s+(?:enfin\s+)?(?:achet[ée]|eu|re[çc]u|trouv[ée]|pris|command[ée]|vu|regard[ée]|fini|termin[ée]|[ée]cout[ée])|passe|mets|marque)\s+(.+?)(?:\s+(?:en|comme)\s+(?:poss[ée]d[ée]e?s?|vu[es]?))?$").captures(raw) {
        let text = m[1].trim().to_string();
        let explicit = crate::re!(r"(?i)\b(?:en|comme)\s+(?:poss[ée]d[ée]e?s?|vu[es]?)\s*$").is_match(raw) || crate::re!(r"(?i)^j['’]ai").is_match(raw);
        let hits = cols.quick_find(&strip_acquisition(&text), None);
        // « J'ai vu Inception » ne fait changer le statut que d'un film « à voir » ; sinon c'est une question (ci-dessous).
        let seen_verb = crate::re!(r"(?i)^j['’]ai\s+(?:enfin\s+)?(?:vu|regard[ée]|fini|termin[ée]|[ée]cout[ée])\b").is_match(raw);
        let still_wanted = hits.first().map(|(l, i)| schema::statut_of(l, i) == Statut::Souhaite).unwrap_or(false);
        if explicit && hits.len() == 1 && (!seen_verb || still_wanted) {
            let (list, item) = hits[0].clone();
            let extras = parse_item_text(&list, &text).values;
            return Some(Intent::new("collection_acquired", "J'ai acheté", &js::gs(&item, "id"), move || {
                let (o, lid, eid, extras) = (oc.clone(), js::gs(&list, "id"), js::gs(&item, "id"), extras.clone());
                async move {
                    let orch = o.up();
                    let Some(l) = orch.collections.get(&lid) else { return say("Cette liste n'existe plus.", "Introuvable") };
                    let skip: Vec<String> = [js::gs(&l, "titre")].into_iter().chain(js::str_list(&l["sousTitre"])).chain(js::str_list(&l["identifiant"])).collect();
                    let values: serde_json::Map<String, Value> = extras.into_iter().filter(|(k, _)| !skip.contains(k)).collect();
                    match orch.collections.set_status(&lid, &eid, true, &Value::Object(values), "voix") {
                        Ok(r) => {
                            let _ = orch.collections.show_list(&lid);
                            say(format!("C'est noté : {} est dans ta liste {}.", js::gs(&r["element"], "titre"), js::gs(&l, "nom")), "✓ possédé")
                        }
                        Err(e) => say(e, "Erreur"),
                    }
                }
            }).free_text());
        }
    }

    // Est-ce que j'ai … ?
    if let Some(m) = crate::re!(&format!(r"(?i)^{ASK}\s+(?:encore\s+)?(.+)$")).captures(raw) {
        let target = strip_seen(m[1].trim());
        let named = cols.list_in_text(&target);
        let hits = cols.quick_find(&target, named.as_ref());
        // On n'intervient que si la phrase parle d'une liste ou si une fiche porte ce nom : « j'ai faim » ne nous regarde pas.
        if named.is_some() || !hits.is_empty() {
            let negated = crate::re!(r"(?i)\b(?:je n['’]ai pas|j['’]ai pas)\b").is_match(raw);
            return Some(Intent::new("collection_have", "Est-ce que j'ai", &target.clone(), move || {
                let (o, target, named_id) = (oc.clone(), target.clone(), named.as_ref().map(|l| js::gs(l, "id")));
                async move {
                    let orch = o.up();
                    let cols = &orch.collections;
                    let named = named_id.and_then(|id| cols.get(&id));
                    let mut hits = cols.quick_find(&target, named.as_ref());
                    if hits.is_empty() {
                        // Pas par le titre : le RAG (sens et mots), seulement pour un résultat sûr.
                        let only = named.as_ref().map(|l| js::gs(l, "id"));
                        hits = cols.search(&target, only.as_deref(), 3).await.into_iter().filter(|(_, _, h)| h.exact || h.score >= 0.8).map(|(l, i, _)| (l, i)).collect();
                    }
                    match hits.len() {
                        0 => {
                            let scope = named.as_ref().map(|l| format!("dans ta liste {}", js::gs(l, "nom"))).unwrap_or_else(|| "dans tes listes".into());
                            if let Some(l) = &named {
                                cols.offer_add(l, &strip_leading_noun_of(l, &target));
                                return say(format!("Non, je ne trouve pas « {} » {scope}. Je l'ajoute à tes souhaits ? Dis « oui » ou « non ».", strip_leading_noun_of(l, &target)), "Pas dans la liste");
                            }
                            say(format!("Non, je ne trouve pas « {target} » {scope}."), "Pas dans la liste")
                        }
                        1 => {
                            let (l, i) = &hits[0];
                            let _ = cols.show_item(&js::gs(l, "id"), &js::gs(i, "id"));
                            let (title, sub) = (schema::title_of(l, i), schema::subtitle_of(l, i));
                            let name = if sub.is_empty() { title } else { format!("{title}, {}", sub.replace(" · ", ", ")) };
                            let text = match schema::statut_of(l, i) {
                                Statut::Souhaite => format!("Pas encore : {name} {}.", js::gs(&schema::vocab(l), "vouluPhrase")),
                                _ => format!("{} {name}.{}", { let yes = js::gs(&schema::vocab(l), "ouiPhrase"); if negated { format!("Si, {} :", yes.trim_start_matches("Oui, ")) } else { format!("{yes} :") } }, { let p = purchase_phrase(l, i); if p.is_empty() { String::new() } else { format!(" {p}") } }),
                            };
                            say(text, schema::title_of(l, i))
                        }
                        n => {
                            let names: Vec<String> = hits.iter().take(5).map(|(l, i)| schema::title_of(l, i)).collect();
                            say(format!("J'en trouve {n} : {}.", names.join(", ")), format!("{n} fiches"))
                        }
                    }
                }
            }).free_text());
        }
    }

    // Affiche-moi mes souhaits (en Lego) / ce qui me manque.
    {
        let wish_list = crate::re!(r"(?i)^(?:affiche|montre|ouvre)(?:[- ]moi)?\s+(?:mes|ma|la)\s+(?:souhaits|liste de souhaits|wishlist|envies)(?:\s+(?:(?:de|en|pour|sur|dans)\s+)?(.+))?$")
            .captures(raw)
            .and_then(|m| m.get(1).and_then(|g| cols.find_by_phrase(g.as_str())))
            .or_else(|| crate::re!(r"(?i)^(?:affiche|montre|ouvre)(?:[- ]moi)?\s+(?:mes|les)\s+(.+?)\s+[àa]\s+(?:voir|regarder|[ée]couter)$").captures(raw).and_then(|m| cols.list_in_text(&m[1])));
        if let Some(list) = wish_list {
            return Some(Intent::new("collection_wishes", "Mes souhaits", &js::gs(&list, "nom"), move || {
                let (o, id) = (oc.clone(), js::gs(&list, "id"));
                async move {
                    let orch = o.up();
                    let Some(l) = orch.collections.get(&id) else { return say("Cette liste n'existe plus.", "Introuvable") };
                    let wishes = js::arr(&l["elements"]).iter().filter(|i| schema::statut_of(&l, i) == Statut::Souhaite).count();
                    let mut card = super::cards::list_card(&l);
                    card["filtre"] = json!("souhaite");
                    orch.show_widget("tool", card);
                    say(if wishes == 0 { format!("Tu n'as aucun souhait dans ta liste {}.", js::gs(&l, "nom")) } else { format!("Voici tes souhaits en {} : {}.", js::gs(&l, "nom"), plural(wishes, "fiche", "fiches")) }, "Souhaits")
                }
            }));
        }
    }

    // Écoute le vinyle … / prix du Lego … / ouvre la fiche Lego du … (boutons spéciaux de la liste).
    if let Some(m) = crate::re!(r"(?i)^(?:[ée]coute|joue|lance|mets)\s+(?:le\s+|l['’]|un\s+)?(?:vinyle|disque|album|33 tours)\s+(.+)$").captures(raw) {
        if let Some((list, item, action)) = action_target(cols, &m[1], |a| js::gs(a, "type") == "spotify") {
            return Some(action_intent(oc, list, item, action, "Écouter"));
        }
    }
    if let Some(m) = crate::re!(r"(?i)^(?:donne[- ]moi|c['’]est quoi|quel est|cherche|trouve(?:[- ]moi)?|v[ée]rifie)\s+(?:le\s+)?prix(?:\s+actuel)?\s+(?:du|de la|de l['’]|des|de)\s+(.+)$").captures(raw) {
        if let Some((list, item, action)) = action_target(cols, &m[1], |a| js::gs(a, "type") == "prix") {
            return Some(action_intent(oc, list, item, action, "Prix actuel"));
        }
    }
    if let Some(m) = crate::re!(r"(?i)^(?:ouvre|affiche|montre)(?:[- ]moi)?\s+(?:la\s+)?(?:fiche|page)\s+(?:officielle\s+)?(\w+)\s+(?:du\s+|de la\s+|de l['’]|des\s+|de\s+)?(.+)$").captures(raw) {
        let key = fold(&m[1]);
        if let Some((list, item, action)) = action_target(cols, &m[2], |a| js::gs(a, "type") == "url" && fold(&js::gs(a, "libelle")).contains(&key)) {
            return Some(action_intent(oc, list, item, action, "Ouvrir"));
        }
    }

    None
}

fn match_open_item(oc: OrchRef, cols: &Collections, raw: &str) -> Option<Intent> {
    // Affiche-moi tel disque / ouvre la fiche de … (une fiche précise, jamais une phrase libre).
    if let Some(m) = crate::re!(r"(?i)^(?:affiche|montre|ouvre|donne|cherche|retrouve|trouve|fais voir)(?:[- ]moi)?\s+(?:la\s+fiche\s+(?:de\s+|du\s+|d['’])?)?(.+)$").captures(raw) {
        let target = m[1].trim().to_string();
        let hits = cols.quick_find(&target, cols.list_in_text(&target).as_ref());
        if !hits.is_empty() && hits.len() <= 8 {
            return Some(Intent::new("collection_open_item", "Ouvrir une fiche", &target.clone(), move || {
                let (o, target) = (oc.clone(), target.clone());
                async move {
                    let orch = o.up();
                    let cols = &orch.collections;
                    let hits = cols.quick_find(&target, cols.list_in_text(&target).as_ref());
                    match hits.len() {
                        0 => say(format!("Je ne trouve pas « {target} »."), "Introuvable"),
                        1 => {
                            let (l, i) = &hits[0];
                            let _ = cols.show_item(&js::gs(l, "id"), &js::gs(i, "id"));
                            say(format!("Voici {}{}.", schema::title_of(l, i), { let s = schema::subtitle_of(l, i); if s.is_empty() { s } else { format!(", {}", s.replace(" · ", ", ")) } }), schema::title_of(l, i))
                        }
                        n => {
                            let (l, _) = &hits[0];
                            let _ = cols.show_list(&js::gs(l, "id"));
                            say(format!("J'en trouve {n} : {}.", hits.iter().take(5).map(|(l, i)| schema::title_of(l, i)).collect::<Vec<_>>().join(", ")), format!("{n} fiches"))
                        }
                    }
                }
            }).free_text());
        }
    }
    None
}

/// « vu Inception » → « Inception » (après « est-ce que j'ai »).
fn strip_seen(s: &str) -> String {
    crate::re!(r"(?i)^(?:vu|vue|vus|vues|regard[ée]e?s?|[ée]cout[ée]e?s?|assist[ée]\s+[àa])\s+").replace(s.trim(), "").to_string()
}

/// « Thriller 25 euros à la Fnac » → « Thriller » (l'identification de la fiche, sans prix ni lieu).
fn strip_acquisition(text: &str) -> String {
    let t = crate::re!(r"(?i)\s*(?:\d+(?:[.,]\d{1,2})?\s*(?:€|euros?|eur)).*$").replace(text, "").to_string();
    let t = crate::re!(r"(?i)\s+(?:chez|[àa]\s+la|[àa]\s+l['’]|au|aux|sur|le\s+\d{1,2}\b|hier|aujourd['’]hui).*$").replace(&t, "").to_string();
    strip_det(&t)
}

fn action_target(cols: &Collections, text: &str, pick: impl Fn(&J) -> bool) -> Option<(J, J, J)> {
    for (list, item) in cols.quick_find(text, None) {
        if let Some(a) = js::arr(&list["actions"]).into_iter().find(|a| pick(a)) {
            return Some((list, item, a));
        }
    }
    None
}

fn action_intent(o: OrchRef, list: J, item: J, action: J, label: &str) -> Intent {
    Intent::new("collection_action", label, &schema::title_of(&list, &item), move || {
        let (o, lid, eid, aid) = (o.clone(), js::gs(&list, "id"), js::gs(&item, "id"), js::gs(&action, "id"));
        async move {
            let orch = o.up();
            let _ = orch.collections.show_item(&lid, &eid);
            match orch.collections.run_action(&lid, &eid, &aid).await {
                Ok(r) => say(js::gs(&r, "say"), "Action"),
                Err(e) => say(e, "Erreur"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistiques_d_une_liste() {
        let (vinyles, _) = lists();
        let st = statistics(&vinyles);
        assert_eq!(st["total"], json!(3));
        assert_eq!(st["possedes"], json!(2));
        assert_eq!(st["souhaites"], json!(1));
        assert_eq!(st["depense"], json!(43.9));
        let artistes = js::arr(&st["repartitions"]).into_iter().find(|g| g["cle"] == "artiste").expect("répartition par artiste");
        assert_eq!(artistes["lignes"][0], json!({ "valeur": "U2", "nombre": 2 }));
    }

    fn lists() -> (J, J) {
        let mut v = schema::normalize_schema(&schema::preset("vinyle").unwrap()).unwrap();
        v["id"] = json!("vinyles_t");
        v["elements"] = json!([
            { "id": "e1", "valeurs": { "titre": "Under a Blood Red Sky", "artiste": "U2", "annee": 1983, "etat": "Possédé", "prix_achat": 18.0, "lieu_achat": "la Fnac", "date_achat": "2024-03-12" } },
            { "id": "e2", "valeurs": { "titre": "Thriller", "artiste": "Michael Jackson", "etat": "Possédé", "prix_achat": 25.9 } },
            { "id": "e3", "valeurs": { "titre": "The Joshua Tree", "artiste": "U2", "etat": "Souhaité" } }
        ]);
        let mut l = schema::normalize_schema(&schema::preset("lego").unwrap()).unwrap();
        l["id"] = json!("lego_t");
        l["elements"] = json!([
            { "id": "l1", "valeurs": { "numero": "75192", "nom": "Faucon Millenium", "etat": "Possédé", "prix_achat": 799.99, "magasin": "Lego Store" } },
            { "id": "l2", "valeurs": { "numero": "75436", "nom": "Faucon Millenium (2025)", "etat": "Souhaité" } }
        ]);
        (v, l)
    }

    #[test]
    fn trouver_une_fiche() {
        let (v, l) = lists();
        let f = |list: &J, t: &str| Collections::find_item(list, t).iter().map(|i| js::gs(i, "id")).collect::<Vec<_>>();
        assert_eq!(f(&l, "le lego 75436"), vec!["l2"]);
        assert_eq!(f(&l, "75192"), vec!["l1"]);
        assert_eq!(f(&v, "le disque Thriller"), vec!["e2"]);
        assert_eq!(f(&v, "thriller de Michael Jackson"), vec!["e2"], "le titre est contenu dans la phrase");
        assert_eq!(f(&v, "blood red"), vec!["e1"]);
        assert!(f(&v, "war").is_empty());
        assert!(f(&l, "le lego 12345").is_empty());
    }

    #[test]
    fn phrase_d_achat() {
        let (v, l) = lists();
        assert_eq!(purchase_phrase(&v, &js::arr(&v["elements"])[0]), "Acheté 18 € à la Fnac le 12/03/2024.");
        assert_eq!(purchase_phrase(&l, &js::arr(&l["elements"])[0]), "Acheté 799,99 € chez Lego Store.");
        assert_eq!(purchase_phrase(&l, &js::arr(&l["elements"])[1]), "");
    }

    #[test]
    fn depenses() {
        let (v, l) = lists();
        assert_eq!(spent(&v), (43.9, 2, 2), "le souhait n'est pas compté");
        assert_eq!(spent(&l), (799.99, 1, 1));
    }

    #[test]
    fn identification_sans_prix_ni_lieu() {
        assert_eq!(strip_acquisition("Thriller 25 euros à la Fnac"), "Thriller");
        assert_eq!(strip_acquisition("le disque Thriller chez Cultura"), "disque Thriller");
        assert_eq!(strip_acquisition("le lego 75436"), "lego 75436");
    }

    #[test]
    fn texte_pour_l_ia() {
        let (v, l) = lists();
        let dir = std::env::temp_dir().join(format!("jarvis-test-query-{}", std::process::id()));
        let svc = Collections::new(std::rc::Weak::<crate::orch::Orchestrator>::new(), dir.clone(), std::rc::Rc::new(|_| {}));
        let _ = (v, l);
        assert!(svc.ai_read_text("", "").contains("aucune liste"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
