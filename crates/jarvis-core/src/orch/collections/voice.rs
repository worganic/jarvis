// Les listes à fiches à la voix : « crée-moi une liste de disques vinyle », « ajoute à ma liste vinyle le disque
// Under a Blood Red Sky du groupe U2 », « affiche ma liste de disques », « retire … de ma liste … ».
// Ce module ne fait que comprendre la phrase : tout ce qui change une liste passe par service.rs.

use std::path::Path;

use serde_json::{json, Map, Value};

use super::super::intent::Intent;
use super::super::shopping;
use super::super::smart_lists::SmartLists;
use super::super::{OrchRef, Up};
use super::cards;
use super::schema::{self, fold};
use super::service::Collections;
use crate::js::{self, J};

/// Conversation en cours (création d'une liste, confirmation d'une suppression).
pub struct Conversation {
    pub at: i64,
    pub stage: Stage,
}

pub enum Stage {
    /// L'IA propose les champs (tâche de fond).
    Thinking { prompt: String, topic: String },
    /// Proposition affichée : schéma complet + clés des champs cochés.
    Proposal { schema: J, keep: Vec<String> },
    /// « Tu confirmes la suppression de ta liste … ? »
    ConfirmDelete { list: String },
    /// « Je l'ajoute à tes souhaits ? » après un « non, je ne le trouve pas ».
    OfferAdd { list: String, text: String },
}

const VERB: &str = r"(?:ajoute|rajoute|mets|met|enregistre|note|inscris|rentre|ajouter)(?:[- ]moi)?";
const LIST_WORD: &str = r"(?:ma|mon|la|le|mes|ta|les)?\s*(?:liste|collection)s?";

/// Ce que contient une phrase d'ajout : valeurs de champs (brutes, validées ensuite) et statut.
#[derive(Default, Debug)]
pub struct Parsed {
    pub values: Map<String, Value>,
    /// « possede » ou « souhaite » si la phrase le dit.
    pub statut: Option<&'static str>,
    /// Le titre est un nom provisoire (« Set 75436 ») que l'enrichissement remplacera.
    pub provisional: bool,
}

fn trim_end_punct(s: &str) -> String {
    crate::re!(r"[\s.!?…]+$").replace(s.trim(), "").to_string()
}

fn first_key(list: &J, prefer: &[&str], typ: &str, avoid: &[&str]) -> Option<String> {
    for p in prefer {
        if schema::field(list, p).is_some() {
            return Some((*p).to_string());
        }
    }
    js::arr(&list["champs"]).iter().find(|f| js::gs(f, "type") == typ && !avoid.iter().any(|a| js::gs(f, "cle").contains(a))).map(|f| js::gs(f, "cle"))
}

fn month_number(name: &str) -> Option<u32> {
    Some(match fold(name).as_str() {
        "janvier" => 1,
        "fevrier" => 2,
        "mars" => 3,
        "avril" => 4,
        "mai" => 5,
        "juin" => 6,
        "juillet" => 7,
        "aout" => 8,
        "septembre" => 9,
        "octobre" => 10,
        "novembre" => 11,
        "decembre" => 12,
        _ => return None,
    })
}

/// « le 12 mars », « le 12/03/2024 », « hier », « aujourd'hui » → date ISO et texte sans la date.
fn extract_date(text: &str) -> (Option<String>, String) {
    let today = js::local_now().date_naive();
    if let Some(m) = crate::re!(r"(?i)\b(?:le\s+)?(\d{1,2})(?:er)?\s+(janvier|f[ée]vrier|mars|avril|mai|juin|juillet|ao[uû]t|septembre|octobre|novembre|d[ée]cembre)(?:\s+(\d{4}))?").captures(text) {
        let (d, mo) = (m[1].parse::<u32>().unwrap_or(0), month_number(&m[2]).unwrap_or(0));
        let mut y: i32 = m.get(3).and_then(|y| y.as_str().parse().ok()).unwrap_or(chrono::Datelike::year(&today));
        let mut date = chrono::NaiveDate::from_ymd_opt(y, mo, d);
        if m.get(3).is_none() && date.map(|dt| dt > today).unwrap_or(false) {
            y -= 1;
            date = chrono::NaiveDate::from_ymd_opt(y, mo, d);
        }
        if let Some(date) = date {
            return (Some(date.format("%Y-%m-%d").to_string()), crate::re!(r"\s+").replace_all(&text.replacen(&m[0], " ", 1), " ").trim().to_string());
        }
    }
    if let Some(m) = crate::re!(r"(?i)\b(?:le\s+)?(\d{1,2}[/.\-]\d{1,2}[/.\-](?:\d{2}|\d{4}))\b").captures(text) {
        if let Some(d) = schema::parse_date(&m[1]) {
            return (Some(d), crate::re!(r"\s+").replace_all(&text.replacen(&m[0], " ", 1), " ").trim().to_string());
        }
    }
    for (word, back) in [("aujourd'hui", 0), ("aujourd’hui", 0), ("hier", 1), ("avant-hier", 2)] {
        if let Some(pos) = text.to_lowercase().find(word) {
            let d = today - chrono::Duration::days(back);
            let mut out = text.to_string();
            out.replace_range(pos..pos + word.len(), " ");
            return (Some(d.format("%Y-%m-%d").to_string()), crate::re!(r"\s+").replace_all(&out, " ").trim().to_string());
        }
    }
    (None, text.to_string())
}

/// Comprend « le disque Under a Blood Red Sky du groupe U2 que j'ai acheté 18 euros à la Fnac le 12 mars ».
pub fn parse_item_text(list: &J, text: &str) -> Parsed {
    let mut p = Parsed::default();
    let mut t = trim_end_punct(text);

    // Souhaité ou possédé.
    let wish = crate::re!(r"(?i)\b(?:que\s+je\s+(?:voudrais|veux|souhaite|aimerais)(?:\s+(?:avoir|acheter|poss[ée]der|voir|regarder|[ée]couter))?|je\s+(?:le|la|les)?\s*(?:voudrais|veux|souhaite|aimerais)(?:\s+bien)?|[àa]\s+(?:acheter|voir|regarder|[ée]couter)|que\s+je\s+n['’]ai\s+pas|que\s+je\s+n['’]ai\s+pas\s+encore|dans\s+mes\s+souhaits|souhait[ée]e?s?|wishlist)\b");
    if wish.is_match(&t) {
        p.statut = Some("souhaite");
        t = wish.replace_all(&t, " ").to_string();
    } else if let Some(m) = crate::re!(r"(?i)\b(?:(?:que\s+)?j['’]ai\s+(?:d[ée]j[àa]\s+)?(?:vu|regard[ée]|[ée]cout[ée])e?s?|d[ée]j[àa]\s+vu[es]?|j['’]y\s+[ée]tais)\b").find(&t) {
        // Film, série, concert : « que j'ai vu » vaut « possédé ».
        p.statut = Some("possede");
        let range = m.range();
        t.replace_range(range, " ");
    } else if crate::re!(r"(?i)\b(?:que\s+j['’]ai(?:\s+achet[ée]e?s?)?|que\s+je\s+poss[èe]de|j['’]ai\s+achet[ée]e?s?)\b").is_match(&t) {
        p.statut = Some("possede");
        t = crate::re!(r"(?i)\b(?:que\s+j['’]ai|que\s+je\s+poss[èe]de|j['’]ai)\b").replace_all(&t, " ").to_string();
    }

    // Prix.
    // Le verbe d'achat reste dans la phrase : il sert ensuite à repérer le lieu (« acheté 18 euros à la Fnac »).
    if let Some(m) = crate::re!(r"(?i)(?:\b(?:pay[ée]e?s?|pour|[àa]|au prix de)\s+)?(\d+(?:[.,]\d{1,2})?)\s*(?:€|euros?|eur)(?:\s*(\d{1,2}))?").captures(&t) {
        if let Some(key) = first_key(list, &["prix_achat", "prix_billet"], "prix", &["actuel", "public"]) {
            let cents = m.get(2).map(|c| format!("{:0<2}", c.as_str())).filter(|_| !m[1].contains(['.', ',']));
            let price = match cents {
                Some(c) => format!("{}.{}", &m[1], c),
                None => m[1].to_string(),
            };
            p.values.insert(key, json!(price));
            t = t.replacen(&m[0], " ", 1);
        }
    }

    // Date.
    if let Some(key) = first_key(list, &["date_achat", "date_vue", "date"], "date", &[]) {
        let (date, rest) = extract_date(&t);
        if let Some(d) = date {
            p.values.insert(key, json!(d));
            t = rest;
        }
    }

    // Lieu : seulement après un verbe d'achat (« acheté à la Fnac »), jusqu'à la fin de la phrase.
    if let Some(key) = first_key(list, &["lieu_achat", "magasin"], "texte", &["nom", "titre", "artiste", "label", "edition", "genre", "theme", "numero"]).filter(|k| ["lieu_achat", "magasin"].contains(&k.as_str())) {
        if let Some(m) = crate::re!(r"(?i)\b(?:achet[ée]e?s?|trouv[ée]e?s?|pris(?:e|es)?|command[ée]e?s?|re[çc]ue?s?)\s+(?:(?:chez|[àa]\s+la|[àa]\s+l['’]|au|aux|[àa]|sur|dans\s+la|dans\s+le|en)\s+)([^,;]+?)\s*$").captures(&t) {
            let place = m[1].trim().trim_start_matches("l'").trim_start_matches("l’").to_string();
            if !place.is_empty() {
                p.values.insert(key, json!(js::capitalize(&place)));
                t = t.replacen(&m[0], " ", 1);
            }
        }
    }
    // Concert : « U2 au Stade de France », « Muse à l'Accor Arena à Paris ».
    if schema::field(list, "salle").is_some() && !p.values.contains_key("salle") {
        if let Some(m) = crate::re!(r"(?i)\s+(?:au|aux|[àa]\s+la|[àa]\s+l['’]|[àa]|chez|dans\s+la|dans\s+le)\s+(.+?)\s*$").captures(&t) {
            let place = m[1].trim().trim_start_matches("l'").trim_start_matches("l’").to_string();
            if !place.is_empty() {
                match crate::re!(r"^(.+?)\s+[àa]\s+(\p{Lu}[\p{L}\- ]*)$").captures(&place) {
                    Some(v) if schema::field(list, "ville").is_some() => {
                        p.values.insert("salle".into(), json!(js::capitalize(v[1].trim())));
                        p.values.insert("ville".into(), json!(v[2].trim()));
                    }
                    _ => {
                        p.values.insert("salle".into(), json!(js::capitalize(&place)));
                    }
                }
                t = t.replacen(&m[0], " ", 1);
            }
        }
    }
    // Il reste parfois « acheté » seul.
    t = crate::re!(r"(?i)\b(?:que\s+j['’]ai\s+)?achet[ée]e?s?\b").replace_all(&t, " ").to_string();

    // Le nom de la chose (« le disque », « le lego »…) ne fait pas partie du titre.
    let mut nouns: Vec<String> = js::str_list(&list["alias"]);
    nouns.extend([js::gs(&list["noms"], "singulier"), js::gs(&list["noms"], "pluriel"), js::gs(list, "nom")]);
    nouns.retain(|n| !n.trim().is_empty());
    nouns.sort_by_key(|n| std::cmp::Reverse(n.len()));
    nouns.dedup();
    let alt = nouns.iter().map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
    t = crate::re!(r"\s+").replace_all(&t, " ").trim().to_string();
    if !alt.is_empty() {
        if let Ok(re) = regex::Regex::new(&format!(r"(?i)^(?:(?:le|la|l['’]|les|un|une|des|du|de la|mon|ma)\s*)?(?:{alt})\s+(?:de\s+|du\s+|d['’]\s*)?")) {
            let stripped = re.replace(&t, "").to_string();
            if !stripped.trim().is_empty() {
                t = stripped;
            }
        }
    }

    // Identifiant (numéro de set, code…).
    let mut id_text = String::new();
    if let Some(id_key) = js::str_list(&list["identifiant"]).first().cloned() {
        if let Some(m) = crate::re!(r"(?i)(?:(?:le|la|un|une|du|de la)\s+)?(?:(?:num[ée]ros?|n[°o]|sets?|r[ée]f[ée]rences?|r[ée]f\.?)\s*(?:n[°o]\s*)?)?(?:#|\b)(\d{4,8}[A-Za-z]?)\b").captures(&t) {
            id_text = m[1].to_string();
            p.values.insert(id_key, json!(m[1]));
            t = t.replacen(&m[0], " ", 1);
        }
    }
    // Année (« Inception 2010 »), sauf quand un numéro identifie déjà la fiche.
    if id_text.is_empty() && schema::field(list, "annee").is_some() && !p.values.contains_key("annee") {
        if let Some(m) = crate::re!(r"\b((?:19|20)\d{2})\b").captures(&t) {
            p.values.insert("annee".into(), json!(m[1]));
            t = t.replacen(&m[0], " ", 1);
        }
    }
    t = crate::re!(r"\(\s*\)").replace_all(&t, " ").to_string();
    t = crate::re!(r"\s+").replace_all(&t, " ").trim().trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '-' | '–' | '—')).trim().to_string();
    t = crate::re!(r"(?i)^(?:de|du|d['’])\s+").replace(&t, "").to_string();

    // Artiste / réalisateur / auteur.
    let mut title = t.clone();
    if schema::field(list, "realisateur").is_some() {
        if let Some(m) = crate::re!(r"(?i)^(.+?)\s+(?:r[ée]alis[ée]\s+par|du\s+r[ée]alisateur|par)\s+(.+)$").captures(&t) {
            title = m[1].trim().to_string();
            p.values.insert("realisateur".into(), json!(m[2].trim()));
        }
    }
    if schema::field(list, "artiste").is_some() && js::gs(list, "titre") != "artiste" {
        if let Some(m) = crate::re!(r"(?i)^(.+?)\s+(?:du\s+groupe|de\s+l['’]artiste|du\s+chanteur|de\s+la\s+chanteuse|du\s+duo|du\s+compositeur|de\s+l['’]interpr[èe]te|par)\s+(.+)$").captures(&t) {
            title = m[1].trim().to_string();
            p.values.insert("artiste".into(), json!(m[2].trim()));
        }
    }
    for sub_key in ["artiste", "realisateur", "auteur"] {
        if schema::field(list, sub_key).is_some() && js::gs(list, "titre") != sub_key && !p.values.contains_key(sub_key) {
            if let Some((a, b)) = title.split_once(" - ").or_else(|| title.split_once(" – ")).or_else(|| title.split_once(" — ")) {
                if !a.trim().is_empty() && !b.trim().is_empty() {
                    let next_title = a.trim().to_string();
                    p.values.insert(sub_key.into(), json!(b.trim()));
                    title = next_title;
                    break;
                }
            }
        }
    }
    let title_key = js::gs(list, "titre");
    if title.trim().is_empty() {
        title = if id_text.is_empty() { String::new() } else { format!("{} {id_text}", js::capitalize(&js::gs(&list["noms"], "singulier"))) };
        p.provisional = !title.is_empty();
    }
    if !title.is_empty() {
        p.values.insert(title_key, json!(title));
    }
    p
}

/// Repère tous les numéros d'identifiant (4 à 8 chiffres, ex. sets Lego) d'un texte, hors dates et prix.
pub fn id_numbers_in(text: &str) -> Vec<String> {
    let (_, without_date) = extract_date(text);
    let without_dates = crate::re!(r"\b\d{1,2}[/.\-]\d{1,2}[/.\-](?:\d{2}|\d{4})\b").replace_all(&without_date, " ");
    let without_prices = crate::re!(r"(?i)\b\d+(?:[.,]\d{1,2})?\s*(?:€|euros?|eur)\b").replace_all(&without_dates, " ");
    crate::re!(r"(?:#|\b)(\d{4,8}[A-Za-z]?)\b")
        .captures_iter(&without_prices)
        .map(|c| c[1].to_string())
        .collect()
}

/// Découpe un texte libre contenant plusieurs éléments (numéros de sets, lignes, éléments séparés par des virgules).
pub fn parse_bulk_text(list: &J, text: &str) -> Vec<Parsed> {
    let has_id = !js::str_list(&list["identifiant"]).is_empty();
    let title_key = js::gs(list, "titre");
    let mut out: Vec<Parsed> = vec![];

    for raw_line in crate::re!(r"[\r\n;]+").split(text) {
        let line = crate::re!(r"^(?:[\s•*–—\-]+|\d{1,3}[.)]\s+)").replace(raw_line.trim(), "").trim().to_string();
        if line.is_empty() {
            continue;
        }
        if has_id {
            let nums = id_numbers_in(&line);
            if nums.len() >= 2 {
                let line_statut = parse_item_text(list, &line).statut;
                for raw_part in crate::re!(r"(?i)\s*(?:,|\bet\b)\s*").split(&line) {
                    let part = match raw_part.split_once(':') {
                        Some((before, after)) if id_numbers_in(before).is_empty() && !after.trim().is_empty() => after.trim(),
                        _ => raw_part.trim(),
                    };
                    if part.is_empty() {
                        continue;
                    }
                    let pnums = id_numbers_in(part);
                    if pnums.len() == 1 {
                        let mut p = parse_item_text(list, part);
                        if p.statut.is_none() {
                            p.statut = line_statut;
                        }
                        if p.values.contains_key(&title_key) {
                            out.push(p);
                        }
                    } else if pnums.len() >= 2 {
                        for n in pnums {
                            let mut p = parse_item_text(list, &n);
                            if p.statut.is_none() {
                                p.statut = line_statut;
                            }
                            if p.values.contains_key(&title_key) {
                                out.push(p);
                            }
                        }
                    }
                }
                continue;
            } else if nums.len() == 1 {
                let cleaned = match line.split_once(':') {
                    Some((before, after)) if id_numbers_in(before).is_empty() && !after.trim().is_empty() => after.trim(),
                    _ => line.as_str(),
                };
                let p = parse_item_text(list, cleaned);
                if p.values.contains_key(&title_key) {
                    out.push(p);
                }
                continue;
            }
        }

        // Liste sans identifiant numérique (ou ligne sans numéro) : découpe par virgule si chaque segment a un titre.
        let parts: Vec<&str> = crate::re!(r",\s+").split(&line).map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        let mut merged: Vec<String> = vec![];
        for part in parts {
            let probe = parse_item_text(list, part);
            if !merged.is_empty() && !probe.values.contains_key(&title_key) {
                if let Some(last) = merged.last_mut() {
                    last.push_str(", ");
                    last.push_str(part);
                }
            } else {
                merged.push(part.to_string());
            }
        }
        for chunk in merged {
            let p = parse_item_text(list, &chunk);
            if p.values.contains_key(&title_key) {
                out.push(p);
            }
        }
    }
    out
}

fn extract_json_items(raw: &str) -> Option<Vec<J>> {
    let t = raw.trim();
    if let (Some(a), Some(b)) = (t.find('['), t.rfind(']')) {
        if b > a {
            if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&t[a..=b]) {
                return Some(arr);
            }
        }
    }
    if let Some(obj) = shopping::read_offers_data(Path::new(""), t) {
        if let Some(arr) = obj["elements"].as_array().or_else(|| obj["items"].as_array()) {
            return Some(arr.clone());
        }
        if obj.is_object() {
            return Some(vec![obj]);
        }
    }
    None
}

/// Lit la réponse JSON de l'IA pour un ajout en lot et la convertit en éléments prêts à valider.
pub fn parse_bulk_json(list: &J, raw: &str, original_text: &str) -> Vec<Parsed> {
    let Some(arr) = extract_json_items(raw) else { return vec![] };
    let id_key = js::str_list(&list["identifiant"]).into_iter().next();
    let title_key = js::gs(list, "titre");
    let fields = js::arr(&list["champs"]);
    let orig_folded = fold(original_text);
    let mut out = vec![];

    for item in arr {
        if let Some(s) = item.as_str() {
            let p = parse_item_text(list, s);
            if p.values.contains_key(&title_key) {
                out.push(p);
            }
            continue;
        }
        let src = if item["valeurs"].is_object() { &item["valeurs"] } else { &item };
        let Some(obj) = src.as_object() else { continue };
        let mut p = Parsed::default();

        for st_val in [item.get("statut"), obj.get("statut"), obj.get("etat")].into_iter().flatten() {
            let sf = fold(&js::s(st_val));
            if matches!(sf.as_str(), "souhaite" | "souhaitee" | "souhaites" | "wishlist" | "a voir" | "a lire" | "a jouer" | "a ecouter" | "a acheter") {
                p.statut = Some("souhaite");
            } else if matches!(sf.as_str(), "possede" | "possedee" | "possedes" | "vu" | "lu" | "joue" | "ecoute" | "achete") {
                p.statut = Some("possede");
            }
        }

        for (k, v) in obj {
            if k == "statut" || v.is_null() || js::s(v).trim().is_empty() {
                continue;
            }
            let kf = fold(k);
            let Some(f) = fields.iter().find(|f| js::gs(f, "cle") == *k || fold(&js::gs(f, "cle")) == kf || fold(&js::gs(f, "libelle")) == kf) else {
                continue;
            };
            let f_key = js::gs(f, "cle");
            if let Ok(nv) = schema::normalize_value(f, v) {
                if !nv.is_null() {
                    p.values.insert(f_key, nv);
                }
            }
        }

        if let Some(ik) = &id_key {
            if let Some(id_val) = p.values.get(ik).map(js::s).filter(|s| !s.trim().is_empty()) {
                let sing = js::capitalize(&js::gs(&list["noms"], "singulier"));
                let prov_title = if sing.is_empty() { id_val.clone() } else { format!("{sing} {id_val}") };
                let cur = p.values.get(&title_key).map(js::s).unwrap_or_default();
                let cf = fold(&cur);
                if cur.trim().is_empty() || cf == fold(&id_val) || cf == fold(&prov_title) {
                    p.values.insert(title_key.clone(), json!(prov_title));
                    p.provisional = true;
                } else if !cf.is_empty() && !orig_folded.contains(&cf) {
                    p.provisional = true;
                }
            }
        }

        if p.values.contains_key(&title_key) {
            out.push(p);
        }
    }
    out
}

fn say(text: impl Into<String>, display: impl Into<String>) -> J {
    json!({ "say": text.into(), "display": display.into() })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n > 1 { many } else { one })
}

/// « 41 possédés et 7 souhaités ».
fn counts_phrase(list: &J) -> String {
    let (total, owned, wish) = schema::counts(list);
    let noun = js::gs(&list["noms"], if total > 1 { "pluriel" } else { "singulier" });
    if total == 0 {
        return "Elle est vide pour l'instant.".into();
    }
    if owned + wish == 0 {
        return format!("Elle contient {total} {noun}.");
    }
    let voc = schema::vocab(list);
    format!("Elle contient {total} {noun} : {} et {}.", plural(owned, &js::gs(&voc, "fait").to_lowercase(), &js::gs(&voc, "faitPluriel").to_lowercase()), plural(wish, &js::gs(&voc, "voulu").to_lowercase(), &js::gs(&voc, "vouluPluriel").to_lowercase()))
}

impl Collections {
    // --- Conversation ----------------------------------------------------------------------------

    pub fn conversation_fresh(&self, ms: i64) -> bool {
        self.conv.borrow().as_ref().map(|c| js::now_ms() - c.at < ms).unwrap_or(false)
    }

    pub fn cancel_conversation(&self) -> bool {
        let had = self.conv.borrow_mut().take().is_some();
        if had {
            self.close_new_card();
        }
        had
    }

    /// Ferme la card de proposition si elle est ouverte.
    fn close_new_card(&self) {
        if let Some(o) = self.o.upgrade() {
            let key = format!("tool:{}", cards::NEW_TITLE);
            if o.cards.has_key(&key) {
                o.emit(json!({ "type": "widget_close", "key": key }));
            }
        }
    }

    /// Phrase d'annonce d'une proposition (lue à voix haute).
    fn proposal_speech(schema_json: &J, keep: &[String]) -> String {
        let title = js::gs(schema_json, "titre");
        let labels: Vec<String> = js::arr(&schema_json["champs"]).iter().filter(|f| keep.contains(&js::gs(f, "cle")) || js::gs(f, "cle") == title).map(|f| js::gs(f, "libelle")).collect();
        let hidden = js::arr(&schema_json["champs"]).len().saturating_sub(labels.len());
        format!(
            "Je te propose la liste « {} » avec les champs : {}.{} Dis « c'est bon » pour la créer, ou par exemple « enlève le prix », « ajoute un champ pays » ou « appelle-la Mes disques ».",
            js::gs(schema_json, "nom"),
            labels.join(", "),
            if hidden > 0 { format!(" J'en garde {hidden} de côté, tu peux les demander.") } else { String::new() }
        )
    }

    fn show_proposal(&self, schema_json: &J, keep: &[String]) {
        self.o.up().show_widget("tool", cards::new_card(schema_json, keep));
    }

    fn set_proposal(&self, schema_json: J, keep: Vec<String>) {
        self.show_proposal(&schema_json, &keep);
        *self.conv.borrow_mut() = Some(Conversation { at: js::now_ms(), stage: Stage::Proposal { schema: schema_json, keep } });
    }

    fn preset_proposal(preset: &J, topic: &str) -> Option<(J, Vec<String>)> {
        let mut schema_json = schema::normalize_schema(preset).ok()?;
        let keep: Vec<String> = js::arr(&preset["champs"]).iter().filter(|f| js::gb(f, "retenu")).map(|f| js::gs(f, "cle")).collect();
        if js::gs(preset, "id") == "libre" && !topic.is_empty() {
            schema_json["nom"] = json!(js::capitalize(&schema::clean_text(topic, 50)));
        }
        Some((schema_json, keep))
    }

    /// Démarre la création : modèle livré si le sujet le connaît, sinon l'IA propose les champs.
    pub fn start_creation(&self, topic_raw: &str) -> J {
        let topic = {
            let t = trim_end_punct(topic_raw);
            let t = crate::re!(r"(?i)^(?:de|d['’]|des|du|pour|sur)\s*").replace(&t, "").to_string();
            schema::clean_text(&t, 80)
        };
        if let Some(preset) = schema::detect_preset(&topic) {
            if let Some(existing) = self.find_by_phrase(&js::gs(&preset, "nom")) {
                let _ = self.show_list(&js::gs(&existing, "id"));
                return say(format!("Tu as déjà une liste « {} ». La voici.", js::gs(&existing, "nom")), js::gs(&existing, "nom"));
            }
            if let Some((s, keep)) = Self::preset_proposal(&preset, &topic) {
                let speech = Self::proposal_speech(&s, &keep);
                self.set_proposal(s.clone(), keep);
                return say(speech, format!("Nouvelle liste : {}", js::gs(&s, "nom")));
            }
        }
        if topic.chars().count() < 2 {
            return say("De quoi veux-tu faire une liste ? Par exemple : une liste de disques vinyle, de livres ou de jeux vidéo.", "Nouvelle liste");
        }
        let prompt = Self::schema_prompt(&topic);
        self.jobs.borrow_mut().push((prompt.clone(), topic.clone()));
        *self.conv.borrow_mut() = Some(Conversation { at: js::now_ms(), stage: Stage::Thinking { prompt: prompt.clone(), topic: topic.clone() } });
        self.o.up().enqueue_ai(prompt, json!({ "background": true }));
        say(format!("Je réfléchis aux champs utiles pour ta liste « {topic} »."), format!("Nouvelle liste : {topic}"))
    }

    fn schema_prompt(topic: &str) -> String {
        let champs = format!("{{\"cle\":\"titre\",\"libelle\":\"Titre\",\"type\":\"{}\"}}", schema::FIELD_TYPES.join("|"));
        [
            format!("L'utilisateur veut créer une liste de collection sur ce sujet : « {topic} »."),
            "Propose les champs à noter pour chaque élément (6 à 12 champs, en français) : identifiant ou numéro s'il y en a un, nom, ce qui caractérise ce sujet, statut possédé ou souhaité, prix payé, date d'achat, lieu d'achat, notes.".into(),
            "N'utilise aucun outil. Réponds uniquement par un objet JSON valide, sans texte autour, de cette forme :".into(),
            format!("{{\"nom\":\"nom court de la liste\",\"icone\":\"un emoji\",\"noms\":{{\"singulier\":\"…\",\"pluriel\":\"…\"}},\"alias\":[\"mots que l'utilisateur emploiera pour désigner la liste\"],\"champs\":[{champs}],\"titre\":\"clé du champ qui sert de nom\",\"sousTitre\":[\"clés de 1 à 2 champs montrés sous le nom\"],\"identifiant\":[\"clé du champ numéro ou code, sinon aucune\"],\"etat\":\"etat\",\"vignette\":[\"clé du champ prix payé\"],\"enrichissement\":{{\"fournisseur\":\"web\"}}}}"),
            "Le champ etat est de type choix avec exactement les choix « Possédé » et « Souhaité ». Un champ de type choix a une liste « choix ».".into(),
            format!("[liste-{}]", js::now_ms()),
        ]
        .join(" ")
    }

    /// Fin de la tâche de fond qui propose les champs : true si c'était la nôtre.
    pub fn done(&self, text: &str, answer: &str, ok: bool) -> bool {
        if self.enrich_done(text, answer, ok) {
            return true;
        }
        let topic = {
            let mut jobs = self.jobs.borrow_mut();
            let Some(i) = jobs.iter().position(|(p, _)| p == text) else { return false };
            jobs.remove(i).1
        };
        let waiting = matches!(self.conv.borrow().as_ref().map(|c| &c.stage), Some(Stage::Thinking { prompt, .. }) if prompt == text);
        if !waiting {
            return true; // annulée entre-temps
        }
        let o = self.o.up();
        let parsed = if ok { shopping::read_offers_data(Path::new(""), answer) } else { None };
        let proposal = parsed.and_then(|d| schema::normalize_schema(&d).ok()).filter(|s| js::arr(&s["champs"]).len() >= 2);
        let (s, intro) = match proposal {
            Some(s) => (s, String::new()),
            None => {
                let libre = schema::preset("libre").unwrap_or(J::Null);
                match Self::preset_proposal(&libre, &topic) {
                    Some((s, _)) => (s, "Je n'ai pas pu interroger l'IA, voici des champs classiques. ".to_string()),
                    None => {
                        self.cancel_conversation();
                        return true;
                    }
                }
            }
        };
        let keep: Vec<String> = js::arr(&s["champs"]).iter().map(|f| js::gs(f, "cle")).take(10).collect();
        let speech = format!("{intro}{}", Self::proposal_speech(&s, &keep));
        self.set_proposal(s, keep);
        o.emit(json!({ "type": "say", "text": speech }));
        true
    }

    /// Création demandée par la card (ou par « c'est bon ») à partir de la proposition en cours.
    pub fn create_from_conversation(&self, a: &J, source: &str) -> Result<J, String> {
        let (mut schema_json, mut keep) = match self.conv.borrow().as_ref().map(|c| &c.stage) {
            Some(Stage::Proposal { schema, keep }) => (schema.clone(), keep.clone()),
            _ => {
                let id = js::gs(a, "modele");
                let preset = schema::preset(&id).ok_or("La proposition a expiré : redemande la liste.")?;
                Self::preset_proposal(&preset, "").ok_or("Modèle invalide.")?
            }
        };
        if !a["retenus"].is_null() {
            keep = js::str_list(&a["retenus"]);
        }
        let name = schema::clean_text(&js::gs(a, "nom"), 60);
        if !name.is_empty() {
            schema_json["nom"] = json!(name);
        }
        let mut fields = js::arr(&schema_json["champs"]);
        for add in js::arr(&a["ajouts"]) {
            if let Some(f) = schema::normalize_field(&add) {
                if !fields.iter().any(|x| js::gs(x, "cle") == js::gs(&f, "cle")) {
                    keep.push(js::gs(&f, "cle"));
                    fields.push(f);
                }
            }
        }
        let title = js::gs(&schema_json, "titre");
        fields.retain(|f| keep.contains(&js::gs(f, "cle")) || js::gs(f, "cle") == title);
        schema_json["champs"] = json!(fields);
        let list = self.create_list(&schema_json, source)?;
        *self.conv.borrow_mut() = None;
        self.close_new_card();
        let _ = self.show_list(&js::gs(&list, "id"));
        Ok(json!({ "liste": schema::public_list(&list, false), "say": Self::created_speech(&list) }))
    }

    fn created_speech(list: &J) -> String {
        let alias = js::str_list(&list["alias"]).into_iter().next().unwrap_or_else(|| js::gs(list, "nom"));
        format!(
            "Ta liste « {} » est prête, avec {} champs. Dis par exemple : « ajoute à ma liste {alias} … », ou « affiche ma liste {alias} ».",
            js::gs(list, "nom"),
            js::arr(&list["champs"]).len()
        )
    }

    /// Réponse d'un utilisateur pendant une conversation (hors commande reconnue).
    pub fn handle_answer(&self, answer: &str) -> Option<J> {
        let stage_kind = self.conv.borrow().as_ref().map(|c| match &c.stage {
            Stage::Thinking { .. } => 0,
            Stage::Proposal { .. } => 1,
            Stage::ConfirmDelete { .. } => 2,
            Stage::OfferAdd { .. } => 3,
        })?;
        let t = fold(answer);
        if crate::re!(r"^(?:annule|annuler|laisse tomber|laisse|stop|non merci|abandonne|oublie(?: ca)?|non)\b").is_match(&t) {
            self.cancel_conversation();
            return Some(json!({ "say": match stage_kind { 2 => "D'accord, je ne supprime rien.", 3 => "D'accord, je n'ajoute rien.", _ => "D'accord, j'abandonne la création de cette liste." }, "done": true }));
        }
        self.conv.borrow_mut().as_mut().map(|c| c.at = js::now_ms());
        match stage_kind {
            0 => Some(json!({ "say": "Je prépare encore les champs, un instant." })),
            2 => {
                let id = match self.conv.borrow().as_ref().map(|c| &c.stage) {
                    Some(Stage::ConfirmDelete { list }) => list.clone(),
                    _ => return None,
                };
                if crate::re!(r"^(?:oui|ouais|confirme|je confirme|vas y|d accord|ok|supprime|efface|c est bon)\b").is_match(&t) {
                    self.cancel_conversation();
                    return Some(match self.delete_list(&id, None, "voix") {
                        Ok(r) => json!({ "say": format!("C'est fait : la liste {} est supprimée. Dis « annule » pour la récupérer.", js::gs(&r, "nom")), "done": true }),
                        Err(e) => json!({ "say": e, "done": true }),
                    });
                }
                Some(json!({ "say": "Dis « oui » pour supprimer la liste, ou « non » pour la garder." }))
            }
            3 => {
                let (lid, text) = match self.conv.borrow().as_ref().map(|c| &c.stage) {
                    Some(Stage::OfferAdd { list, text }) => (list.clone(), text.clone()),
                    _ => return None,
                };
                if crate::re!(r"^(?:oui|ouais|vas y|d accord|ok|ajoute(?: le| la)?|c est bon|s il te plait)").is_match(&t) {
                    self.cancel_conversation();
                    let list = self.get(&lid)?;
                    return Some(self.add_spoken(&list, &format!("{text} que je voudrais")));
                }
                Some(json!({ "say": "Dis « oui » pour l'ajouter à tes souhaits, ou « non »." }))
            }
            _ => self.edit_proposal(answer, &t),
        }
    }

    fn edit_proposal(&self, raw: &str, t: &str) -> Option<J> {
        let (mut schema_json, mut keep) = match self.conv.borrow().as_ref().map(|c| &c.stage) {
            Some(Stage::Proposal { schema, keep }) => (schema.clone(), keep.clone()),
            _ => return None,
        };
        let title = js::gs(&schema_json, "titre");
        let fields = js::arr(&schema_json["champs"]);
        let find = |name: &str| -> Option<String> {
            let n = fold(name);
            if n.is_empty() {
                return None;
            }
            let ln = super::service::loose(&n);
            fields
                .iter()
                .find(|f| fold(&js::gs(f, "libelle")) == n || js::gs(f, "cle") == n.replace(' ', "_") || super::service::loose(&js::gs(f, "libelle")) == ln)
                .or_else(|| fields.iter().find(|f| fold(&js::gs(f, "libelle")).split(' ').any(|w| w == n) || fold(&js::gs(f, "libelle")).contains(&n) || n.contains(&fold(&js::gs(f, "libelle")))))
                .map(|f| js::gs(f, "cle"))
        };
        let names = |s: &str| -> Vec<String> {
            crate::re!(r"(?i)\s*(?:,|;|\bet\b|\bpuis\b|\bainsi que\b)\s*").split(s.trim()).map(|x| x.trim().trim_start_matches(|c: char| c == '-' || c == ' ').to_string()).filter(|x| !x.is_empty()).collect()
        };
        let mut said = String::new();
        let mut changed = true;

        if crate::re!(r"^(?:c est bon|ok|d accord|oui|valide|valider|cree|creer|vas y|go|parfait|ca me va|ca marche|c est parfait|c est bien|confirme|c est ok|nickel|impeccable)(?: la| ca| le| liste| la liste| merci| stp| s il te plait)*$").is_match(t) {
            let a = json!({});
            return Some(match self.create_from_conversation(&a, "voix") {
                Ok(r) => json!({ "say": r["say"], "done": true }),
                Err(e) => json!({ "say": e, "done": true }),
            });
        }
        if let Some(m) = crate::re!(r"(?i)^(?:appelle|nomme|renomme|intitule|intitulé)(?:[- ]la|[- ]le)?(?:\s+liste)?\s+(.+)$|^(?:le nom|elle s['’]appelle|ça s['’]appelle|ca s['’]appelle)\s+(?:c['’]est|sera|est)?\s*(.+)$").captures(raw.trim()) {
            let name = trim_end_punct(m.get(1).or(m.get(2)).map(|x| x.as_str()).unwrap_or(""));
            let name = schema::clean_text(name.trim_matches(|c| c == '«' || c == '»' || c == '"'), 60);
            if !name.is_empty() {
                schema_json["nom"] = json!(name);
                said = format!("D'accord, la liste s'appellera « {name} ».");
            }
        } else if let Some(m) = crate::re!(r"^(?:garde|gardes|conserve|prends|je veux|mets)(?: moi)? (?:seulement|uniquement|juste|que|rien que)(?: les champs| le champ| ceux)? (.+)$").captures(t) {
            let wanted: Vec<String> = names(&m[1]).iter().filter_map(|n| find(n)).collect();
            if wanted.is_empty() {
                said = "Je n'ai reconnu aucun de ces champs.".into();
                changed = false;
            } else {
                keep = wanted;
                said = "C'est noté.".into();
            }
        } else if let Some(m) = crate::re!(r"^(?:enleve|retire|supprime|vire|ote|sans|pas de|pas besoin d[eu]?s?)(?: moi)?(?: le champ| les champs| la colonne| le| la| les| l| du| de la| des)? ?(.+)$").captures(t) {
            let mut gone = vec![];
            let mut unknown = vec![];
            for n in names(&m[1]) {
                match find(&n) {
                    Some(k) if k == title => said = format!("Le champ « {} » sert de nom, je le garde.", js::gs(fields.iter().find(|f| js::gs(f, "cle") == k).unwrap_or(&J::Null), "libelle")),
                    Some(k) => {
                        keep.retain(|x| *x != k);
                        gone.push(k);
                    }
                    None => unknown.push(n),
                }
            }
            if !gone.is_empty() {
                said = format!("{} retiré{}.", plural(gone.len(), "champ", "champs"), if gone.len() > 1 { "s" } else { "" });
            } else if said.is_empty() {
                said = format!("Je ne trouve pas le champ {}.", unknown.join(", "));
                changed = false;
            }
        } else if let Some(m) = crate::re!(r"^(?:ajoute|rajoute|mets|je veux aussi|ajoute aussi)(?: moi)?(?: encore)?(?: un| une| le| la| les| des)?(?: nouveau| nouvelle)?(?: champ| colonne| info| information)?(?: pour| sur| de| d| nomme| appele| appelle)? ?(.+)$").captures(t) {
            let mut added = vec![];
            for n in names(&m[1]) {
                if let Some(k) = find(&n) {
                    if !keep.contains(&k) {
                        keep.push(k);
                        added.push(n);
                    }
                    continue;
                }
                if js::arr(&schema_json["champs"]).len() >= schema::MAX_FIELDS {
                    said = format!("Une liste ne peut pas avoir plus de {} champs.", schema::MAX_FIELDS);
                    break;
                }
                let label = js::capitalize(&n);
                let mut fs = js::arr(&schema_json["champs"]);
                let key = {
                    let base = schema::slug(&label);
                    let mut k = base.clone();
                    let mut i = 2;
                    while fs.iter().any(|f| js::gs(f, "cle") == k) {
                        k = format!("{base}_{i}");
                        i += 1;
                    }
                    k
                };
                fs.push(json!({ "cle": key, "libelle": label, "type": guess_type(&n) }));
                schema_json["champs"] = json!(fs);
                keep.push(key);
                added.push(n);
            }
            if said.is_empty() {
                said = if added.is_empty() { "Rien à ajouter.".into() } else { format!("J'ajoute {}.", added.join(", ")) };
            }
        } else if crate::re!(r"^(?:quels|quelles|lis|redis|repete|rappelle)\b").is_match(t) {
            said = Self::proposal_speech(&schema_json, &keep);
            changed = false;
        } else {
            return Some(json!({ "say": "Dis « c'est bon » pour créer la liste, ou par exemple « enlève le prix », « garde seulement le titre et l'artiste », « ajoute un champ pays ». Dis « annule » pour abandonner." }));
        }
        if changed {
            self.set_proposal(schema_json, keep);
        }
        Some(json!({ "say": said }))
    }

    /// Demande venue de la card : « + Nouvelle liste » (modèle livré ou sujet libre).
    pub async fn propose(&self, a: &J) -> Result<J, String> {
        let model = js::gs(a, "modele");
        if !model.is_empty() {
            let preset = schema::preset(&model).ok_or("Modèle inconnu.")?;
            if let Some(existing) = self.find_by_phrase(&js::gs(&preset, "nom")) {
                let _ = self.show_list(&js::gs(&existing, "id"));
                return Ok(json!({ "say": format!("Tu as déjà une liste « {} ».", js::gs(&existing, "nom")) }));
            }
            let (s, keep) = Self::preset_proposal(&preset, "").ok_or("Modèle invalide.")?;
            let speech = Self::proposal_speech(&s, &keep);
            self.set_proposal(s, keep);
            return Ok(json!({ "say": speech }));
        }
        let topic = js::gs(a, "sujet");
        if topic.trim().is_empty() {
            return Err("Dis de quoi doit parler la liste.".into());
        }
        Ok(self.start_creation(&topic))
    }

    // --- Suppression confirmée à la voix -------------------------------------------------------------

    fn ask_delete(&self, list: &J) -> J {
        let (total, _, _) = schema::counts(list);
        *self.conv.borrow_mut() = Some(Conversation { at: js::now_ms(), stage: Stage::ConfirmDelete { list: js::gs(list, "id") } });
        say(
            format!("Tu confirmes la suppression de ta liste « {} »{} ? Dis « oui » ou « non ».", js::gs(list, "nom"), if total > 0 { format!(" et de ses {total} éléments") } else { String::new() }),
            "Supprimer la liste ?",
        )
    }

    /// Ajout dicté : crée l'élément, ou signale le doublon.
    pub fn add_spoken(&self, list: &J, item_text: &str) -> J {
        let parsed = parse_item_text(list, item_text);
        let list_id = js::gs(list, "id");
        let opts = json!({ "statut": parsed.statut.unwrap_or(""), "provisoire": parsed.provisional });
        let values = Value::Object(parsed.values.clone());
        if !parsed.values.contains_key(&js::gs(list, "titre")) {
            return say(format!("Je n'ai pas compris quel {} ajouter. Dis par exemple : ajoute à ma liste {} …", js::gs(&list["noms"], "singulier"), js::gs(list, "nom")), "Rien à ajouter");
        }
        match self.add_item(&list_id, &values, &opts, "voix") {
            Ok(r) if js::gb(&r, "doublon") => {
                let existing = r["element"].clone();
                let id = js::gs(&existing, "id");
                let title = js::gs(&existing, "titre");
                if js::gs(&existing, "statut") == "souhaite" && (parsed.statut == Some("possede") || (parsed.statut.is_none() && js::gs(&schema::vocab(list), "defaut") == "fait")) {
                    let extra = Value::Object(parsed.values.iter().filter(|(k, _)| **k != js::gs(list, "titre") && !js::str_list(&list["sousTitre"]).contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect());
                    return match self.set_status(&list_id, &id, true, &extra, "voix") {
                        Ok(_) => {
                            let _ = self.show_list(&list_id);
                            say(format!("« {title} » était dans tes souhaits : je l'ai passé en possédé."), format!("✓ {title}"))
                        }
                        Err(e) => say(e, "Erreur"),
                    };
                }
                let _ = self.show_item(&list_id, &id);
                say(format!("« {title} » est déjà dans ta liste {} : {}.", js::gs(list, "nom"), super::service::statut_phrase(list, match js::gs(&existing, "statut").as_str() {
                    "possede" => schema::Statut::Possede,
                    "souhaite" => schema::Statut::Souhaite,
                    _ => schema::Statut::Inconnu,
                })), "Déjà dans la liste")
            }
            Ok(r) => {
                let e = &r["element"];
                let sub = js::gs(e, "sousTitre");
                let wish = js::gs(e, "statut") == "souhaite";
                let voulu = js::gs(&schema::vocab(list), "voulu");
                let wish_label = if voulu == "Souhaité" { "Ajouté à tes souhaits".to_string() } else { format!("Ajouté, {}", voulu.to_lowercase()) };
                let _ = self.show_list(&list_id);
                say(
                    format!("{} à ta liste {} : {}{}.", if wish { wish_label.as_str() } else { "Ajouté" }, js::gs(list, "nom"), js::gs(e, "titre"), if sub.is_empty() { String::new() } else { format!(", {}", sub.replace(" · ", ", ")) }),
                    format!("+ {}", js::gs(e, "titre")),
                )
            }
            Err(e) => say(e, "Erreur"),
        }
    }
}

pub(super) fn guess_type(label: &str) -> &'static str {
    let t = fold(label);
    let has = |words: &[&str]| words.iter().any(|w| t.split(' ').any(|x| x == *w || x.starts_with(w)));
    if has(&["prix", "cout", "montant", "tarif", "valeur"]) {
        "prix"
    } else if has(&["annee", "nombre", "numero", "quantite", "pieces", "duree", "poids", "taille", "age", "note"]) && !has(&["notes"]) {
        "nombre"
    } else if has(&["date", "jour", "quand"]) {
        "date"
    } else if has(&["lien", "site", "url"]) {
        "lien"
    } else if has(&["photo", "image", "pochette", "couverture"]) {
        "image"
    } else if has(&["notes", "commentaire", "description", "avis", "remarque"]) {
        "texte_long"
    } else {
        "texte"
    }
}

fn not_a_list_topic(topic: &str) -> bool {
    crate::re!(r"\b(?:notes?|reunions?|visios?|projets?|mails?|emails?|rappels?|applications?|applis?|fenetres?|fichiers?|automatisations?|raccourcis?|commandes?|gmeet|musiques?|playlists?)\b").is_match(&fold(topic))
}

/// Intentions des listes à fiches : placées AVANT celles des listes à cocher.
pub fn match_collection(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    let orch = o.up();
    match_with(o, &orch.collections, t, raw)
}

fn match_with(o: &OrchRef, cols: &Collections, t: &str, raw: &str) -> Option<Intent> {
    let oc = o.clone();
    let raw = trim_end_punct(if raw.is_empty() { t } else { raw });

    // Toutes mes listes (à fiches et à cocher), y compris « ouvre la card/carte/module liste(s)/list ».
    if crate::re!(r"^(?:quelles sont mes listes|liste (?:mes|toutes mes|les) listes|mes listes|toutes mes listes|(?:affiche|affichez|afficher|montre|montrez|montrer|ouvre|ouvrez|ouvrir|lance|lancez|lancer|demarre|demarrer|active|activer|fais voir)(?: moi)? (?:(?:mes|toutes mes|les) (?:listes?|lists?|collections?)|(?:la |le |les |mes )?(?:card|cards|carte|cartes|carde|cardes|module|modules|widget|widgets|panneau|fenetre|fiche) (?:de |des |d |mes )?(?:listes?|lists?|collections?)(?: \d{1,3})?)|(?:la |le )?(?:card|cards|carte|cartes|carde|cardes|module|modules) (?:de |des |d |mes )?(?:listes?|lists?|collections?)(?: \d{1,3})?)(?: s il te plait| s il vous plait| stp| svp| merci)?$").is_match(t) {
        return Some(Intent::new("collections_overview", "Mes listes", "toutes les listes", move || {
            let o = oc.clone();
            async move {
                let orch = o.up();
                orch.collections.show_overview();
                let (f, c) = (orch.collections.count(), orch.smart_lists.count());
                let names: Vec<String> = orch.collections.all().iter().map(|l| js::gs(l, "nom")).collect();
                let text = match (f, c) {
                    (0, 0) => "Tu n'as encore aucune liste. Dis par exemple : crée-moi une liste de disques vinyle.".to_string(),
                    (0, _) => format!("Tu as {}.", plural(c, "liste à cocher", "listes à cocher")),
                    (_, 0) => format!("Tu as {} : {}.", plural(f, "liste", "listes"), names.join(", ")),
                    _ => format!("Tu as {} : {}, et {}.", plural(f, "liste à fiches", "listes à fiches"), names.join(", "), plural(c, "liste à cocher", "listes à cocher")),
                };
                json!({ "say": text, "display": format!("{} listes", f + c) })
            }
        }));
    }

    // Créer une liste.
    if let Some(m) = crate::re!(r"(?i)^(?:(?:jarvis[, ]+)?(?:peux[- ]tu|pourrais[- ]tu|voudrais[- ]tu)\s+)?(?:me\s+)?(?:(?:cr[ée]{1,2}|cr[ée]er|fais|fait|pr[ée]pare|pr[ée]parer|initialise|monte|d[ée]marre|nouvelle)(?:[- ]moi)?(?:\s+(?:une|la))?(?:\s+nouvelle)?|je veux(?:\s+(?:cr[ée]er|faire))?(?:\s+une)?|j['’]aimerais(?:\s+(?:cr[ée]er|faire))?(?:\s+une)?)\s+(?:liste|collection)(?:\s+(?:de|d['’]|des|du|pour|sur)\s*|\s+)(.+)$").captures(&raw) {
        let topic = m[1].trim().to_string();
        if not_a_list_topic(&topic) {
            return None;
        }
        // Les listes à cocher connues (courses, valise, bricolage, tâches, cadeaux) restent aux listes à cocher.
        if schema::detect_preset(&topic).is_none() && SmartLists::detect_type(&topic) != "custom" {
            return None;
        }
        return Some(Intent::new("collection_create", "Créer une liste", &topic.clone(), move || {
            let (o, topic) = (oc.clone(), topic.clone());
            async move {
                let res = o.up().collections.start_creation(&topic);
                json!({ "say": res["say"], "display": res["display"], "quiet": true })
            }
        }).free_text());
    }

    // Requêtes, statuts, champs, actions (avant l'ajout : « ajoute un champ pays à ma liste vinyle » n'est pas une fiche).
    if let Some(i) = super::query::match_query(o, cols, &raw, false) {
        return Some(i);
    }

    // Ajouter un élément : « ajoute à ma liste vinyle le disque X du groupe Y » / « ajoute X à ma liste vinyle ».
    if let Some(m) = crate::re!(&format!(r"(?i)^{VERB}\s+(?:(?:[àa]|dans|sur|en)\s+){LIST_WORD}\s+(.+)$")).captures(&raw) {
        if let Some((list, rest)) = cols.find_by_prefix(&strip_of(&m[1])) {
            if !rest.trim().is_empty() {
                return Some(add_intent(oc, list, rest));
            }
        }
    }
    if let Some(m) = crate::re!(&format!(r"(?i)^{VERB}\s+(.+?)\s+(?:[àa]|dans|sur|en)\s+{LIST_WORD}(?:\s+(?:de|d['’]|des|du|pour)\s*|\s+)(.+)$")).captures(&raw) {
        if let Some(list) = cols.find_by_phrase(&m[2]) {
            return Some(add_intent(oc, list, m[1].trim().to_string()));
        }
    }

    // Retirer un élément : « retire X de ma liste vinyle ».
    if let Some(m) = crate::re!(&format!(r"(?i)^(?:supprime|efface|retire|enl[èe]ve|ote)(?:[- ]moi)?\s+(.+?)\s+(?:de|dans|sur|d['’])\s*{LIST_WORD}(?:\s+(?:de|d['’]|des|du|pour)\s*|\s+)(.+)$")).captures(&raw) {
        if let Some(list) = cols.find_by_phrase(&m[2]) {
            let item = strip_leading_noun(&list, m[1].trim());
            return Some(Intent::new("collection_remove_item", "Retirer de la liste", &item.clone(), move || {
                let (o, list_id, item) = (oc.clone(), js::gs(&list, "id"), item.clone());
                async move {
                    let orch = o.up();
                    let Some(list) = orch.collections.get(&list_id) else { return say("Cette liste n'existe plus.", "Liste introuvable") };
                    let found = Collections::find_item(&list, &item);
                    match found.len() {
                        0 => say(format!("Je ne trouve pas « {item} » dans ta liste {}.", js::gs(&list, "nom")), "Élément introuvable"),
                        1 => match orch.collections.delete_item(&list_id, &js::gs(&found[0], "id"), "voix") {
                            Ok(r) => say(format!("J'ai retiré {} de ta liste {}. Dis « annule » pour le remettre.", js::gs(&r, "titre"), js::gs(&list, "nom")), format!("− {}", js::gs(&r, "titre"))),
                            Err(e) => say(e, "Erreur"),
                        },
                        n => say(format!("J'ai trouvé {n} éléments qui correspondent à « {item} » : sois plus précis."), "Plusieurs correspondances"),
                    }
                }
            }).free_text());
        }
    }

    // Supprimer une liste (confirmation orale).
    if let Some(m) = crate::re!(r"(?i)^(?:supprime|efface|retire|d[ée]truis)(?:[- ]moi)?\s+(?:la|ma|mes|ta)\s+(?:liste|collection)s?(?:\s+(?:de|d['’]|des|du|pour)\s*|\s+)(.+)$").captures(&raw) {
        if let Some(list) = cols.find_by_phrase(&m[1]) {
            return Some(Intent::new("collection_delete", "Supprimer une liste", &js::gs(&list, "nom"), move || {
                let (o, list_id) = (oc.clone(), js::gs(&list, "id"));
                async move {
                    let orch = o.up();
                    match orch.collections.get(&list_id) {
                        Some(l) => orch.collections.ask_delete(&l),
                        None => say("Cette liste n'existe plus.", "Liste introuvable"),
                    }
                }
            }));
        }
    }

    // Afficher une liste : « affiche ma liste de disques », « montre mes lego ».
    let show = crate::re!(r"(?i)^(?:affiche|montre|ouvre|donne|consulte|lis|fais voir|fais[- ]moi voir)(?:[- ]moi)?\s+(?:(?:la|ma|mes|ta|les|le|mon)\s+)?(?:(?:liste|collection)s?(?:\s+(?:de|d['’]|des|du|pour)\s*|\s+)|(?:card|fen[êe]tre)\s+)?(.+)$");
    if let Some(m) = show.captures(&raw) {
        let has_list_word = crate::re!(r"(?i)\b(?:liste|collection)s?\b").is_match(&raw);
        let target = m[1].trim().to_string();
        if let Some(list) = cols.find_by_phrase(&target) {
            // Sans le mot « liste », seul le nom exact d'une liste compte (« montre mes lego »), jamais une phrase libre.
            let exact = js::str_list(&list["alias"]).iter().chain([&js::gs(&list, "nom")]).any(|a| super::service::loose(a) == super::service::loose(&strip_leading_det(&target)));
            if has_list_word || exact {
                return Some(Intent::new("collection_show", "Ouvrir une liste", &js::gs(&list, "nom"), move || {
                    let (o, list_id) = (oc.clone(), js::gs(&list, "id"));
                    async move {
                        let orch = o.up();
                        match orch.collections.get(&list_id) {
                            Some(l) => {
                                let _ = orch.collections.show_list(&list_id);
                                say(format!("Voici ta liste {}. {}", js::gs(&l, "nom"), counts_phrase(&l)), js::gs(&l, "nom"))
                            }
                            None => say("Cette liste n'existe plus.", "Liste introuvable"),
                        }
                    }
                }));
            }
        }
    }
    // « Affiche-moi Thriller » : une fiche précise, après que les noms de listes ont été essayés.
    super::query::match_query(o, cols, &raw, true)
}

fn strip_leading_det(s: &str) -> String {
    crate::re!(r"(?i)^(?:(?:la|le|les|ma|mon|mes|ta|ton|tes|de|des|du)\s+|l['’]|d['’])").replace(s.trim(), "").to_string()
}

/// « de disques … » après « liste » : on enlève le « de ».
fn strip_of(s: &str) -> String {
    crate::re!(r"(?i)^(?:de|d['’]|des|du|pour)\s*").replace(s.trim(), "").to_string()
}

fn strip_leading_noun(list: &J, item: &str) -> String {
    let mut nouns: Vec<String> = js::str_list(&list["alias"]);
    nouns.extend([js::gs(&list["noms"], "singulier"), js::gs(&list["noms"], "pluriel")]);
    nouns.sort_by_key(|n| std::cmp::Reverse(n.len()));
    let alt = nouns.iter().filter(|n| !n.is_empty()).map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
    if alt.is_empty() {
        return item.to_string();
    }
    match regex::Regex::new(&format!(r"(?i)^(?:(?:le|la|l['’]|les|un|une|des|du|mon|ma)\s*)?(?:{alt})\s+")) {
        Ok(re) => {
            let s = re.replace(item, "").to_string();
            if s.trim().is_empty() { item.to_string() } else { s }
        }
        Err(_) => item.to_string(),
    }
}

fn add_intent(o: OrchRef, list: J, item_text: String) -> Intent {
    Intent::new("collection_add", "Ajouter à la liste", &format!("{item_text} → {}", js::gs(&list, "nom")), move || {
        let (o, list_id, text) = (o.clone(), js::gs(&list, "id"), item_text.clone());
        async move {
            let orch = o.up();
            match orch.collections.get(&list_id) {
                Some(list) => orch.collections.add_spoken(&list, &text),
                None => say("Cette liste n'existe plus.", "Liste introuvable"),
            }
        }
    })
    .free_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(preset: &str) -> J {
        let mut s = schema::normalize_schema(&schema::preset(preset).unwrap()).unwrap();
        s["id"] = json!(format!("{preset}_test"));
        s["elements"] = json!([]);
        s
    }

    fn v(p: &Parsed, k: &str) -> String {
        js::s(p.values.get(k).unwrap_or(&Value::Null))
    }

    #[test]
    fn disque_et_groupe() {
        let p = parse_item_text(&list("vinyle"), "le disque Under a Blood Red Sky du groupe U2");
        assert_eq!(v(&p, "titre"), "Under a Blood Red Sky");
        assert_eq!(v(&p, "artiste"), "U2");
        assert_eq!(p.statut, None);
    }

    #[test]
    fn achat_complet() {
        let p = parse_item_text(&list("vinyle"), "l'album Joshua Tree de U2 acheté 18 euros à la Fnac de Strasbourg le 12 mars 2024.");
        assert_eq!(v(&p, "prix_achat"), "18");
        assert_eq!(v(&p, "date_achat"), "2024-03-12");
        assert_eq!(v(&p, "lieu_achat"), "Fnac de Strasbourg");
        assert!(v(&p, "titre").starts_with("Joshua Tree"), "{:?}", p.values);
    }

    #[test]
    fn prix_avec_centimes() {
        let p = parse_item_text(&list("vinyle"), "le disque Thriller de Michael Jackson, 25,90 €");
        assert_eq!(v(&p, "prix_achat"), "25,90");
    }

    #[test]
    fn souhaite() {
        let p = parse_item_text(&list("vinyle"), "le disque The Wall de Pink Floyd que je voudrais");
        assert_eq!(p.statut, Some("souhaite"));
        assert_eq!(v(&p, "titre"), "The Wall de Pink Floyd");
        let q = parse_item_text(&list("lego"), "le lego 75436 que je veux");
        assert_eq!(q.statut, Some("souhaite"));
        assert_eq!(v(&q, "numero"), "75436");
    }

    #[test]
    fn film_serie_concert() {
        let film = list("film");
        let p = parse_item_text(&film, "le film Inception réalisé par Christopher Nolan 2010 que j'ai vu hier");
        assert_eq!(v(&p, "titre"), "Inception");
        assert_eq!(v(&p, "realisateur"), "Christopher Nolan");
        assert_eq!(v(&p, "annee"), "2010");
        assert_eq!(p.statut, Some("possede"));
        assert!(p.values.contains_key("date_vue"), "{:?}", p.values);
        let a_voir = parse_item_text(&film, "le film Dune à voir");
        assert_eq!(a_voir.statut, Some("souhaite"));
        assert_eq!(v(&a_voir, "titre"), "Dune");

        let serie = list("serie");
        let p = parse_item_text(&serie, "la série Breaking Bad que je voudrais voir");
        assert_eq!(p.statut, Some("souhaite"));
        assert_eq!(v(&p, "titre"), "Breaking Bad");

        let concert = list("concert");
        let p = parse_item_text(&concert, "U2 au Stade de France à Paris le 12 juin 2024");
        assert_eq!(v(&p, "artiste"), "U2");
        assert_eq!(v(&p, "salle"), "Stade de France");
        assert_eq!(v(&p, "ville"), "Paris");
        assert!(v(&p, "date").ends_with("-06-12"), "{:?}", p.values);
        let p = parse_item_text(&concert, "Muse billet 65 euros");
        assert_eq!(v(&p, "prix_billet"), "65");
    }

    #[test]
    fn lego_par_son_numero() {
        let p = parse_item_text(&list("lego"), "le lego 75192 Faucon Millenium acheté 799 euros chez Lego Store");
        assert_eq!(v(&p, "numero"), "75192");
        assert_eq!(v(&p, "nom"), "Faucon Millenium");
        assert_eq!(v(&p, "prix_achat"), "799");
        assert_eq!(v(&p, "magasin"), "Lego Store");
        let seul = parse_item_text(&list("lego"), "le lego 75436");
        assert_eq!(v(&seul, "numero"), "75436");
        assert_eq!(v(&seul, "nom"), "Set 75436", "un numéro seul donne un nom provisoire");
    }

    #[test]
    fn dates_parlees() {
        let (d, rest) = extract_date("acheté hier à la Fnac");
        assert!(d.is_some());
        assert_eq!(rest, "acheté à la Fnac");
        let (d, _) = extract_date("le 5/1/24");
        assert_eq!(d.as_deref(), Some("2024-01-05"));
        assert!(extract_date("rien du tout").0.is_none());
    }

    #[test]
    fn types_devines() {
        assert_eq!(guess_type("Prix de vente"), "prix");
        assert_eq!(guess_type("Date de sortie"), "date");
        assert_eq!(guess_type("Année"), "nombre");
        assert_eq!(guess_type("Pays"), "texte");
        assert_eq!(guess_type("Notes"), "texte_long");
        assert_eq!(guess_type("Photo"), "image");
    }

    fn svc_with_lists() -> (Collections, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("jarvis-test-voice-{}-{}", std::process::id(), js::rand_below(1_000_000)));
        let svc = Collections::new(std::rc::Weak::<crate::orch::Orchestrator>::new(), dir.clone(), std::rc::Rc::new(|_| {}));
        for m in ["vinyle", "lego"] {
            let r = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(tokio::task::LocalSet::new().run_until(svc.dispatch("creer_liste", &schema::preset(m).unwrap(), "test")));
            assert!(js::gb(&r, "ok"), "{r}");
        }
        let (vid, lid) = (svc.all()[0]["id"].as_str().unwrap().to_string(), svc.all()[1]["id"].as_str().unwrap().to_string());
        let run = |op: &str, a: J| tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(tokio::task::LocalSet::new().run_until(svc.dispatch(op, &a, "test")));
        assert!(js::gb(&run("ajouter_element", json!({ "liste": vid, "valeurs": { "titre": "Thriller", "artiste": "Michael Jackson" }, "statut": "souhaite" })), "ok"));
        assert!(js::gb(&run("ajouter_element", json!({ "liste": lid, "valeurs": { "numero": "75436", "nom": "Faucon Millenium" } })), "ok"));
        for m in ["film", "serie", "concert"] {
            assert!(js::gb(&run("creer_liste", schema::preset(m).unwrap()), "ok"));
        }
        let fid = svc.all()[2]["id"].as_str().unwrap().to_string();
        assert!(js::gb(&run("ajouter_element", json!({ "liste": fid, "valeurs": { "titre": "Inception" }, "statut": "possede" })), "ok"));
        assert!(js::gb(&run("ajouter_element", json!({ "liste": fid, "valeurs": { "titre": "Dune" } })), "ok"), "ajouté « à voir » par défaut");
        (svc, dir)
    }

    fn kind(svc: &Collections, raw: &str) -> Option<String> {
        let weak = std::rc::Weak::<crate::orch::Orchestrator>::new();
        match_with(&weak, svc, &fold(raw), raw).map(|i| i.kind)
    }

    #[test]
    fn les_phrases_sont_routees_vers_les_bonnes_intentions() {
        let (svc, dir) = svc_with_lists();
        let k = |s: &str| kind(&svc, s);
        // Création
        assert_eq!(k("Crée-moi une liste de disques vinyle").as_deref(), Some("collection_create"));
        assert_eq!(k("Jarvis, peux-tu me créer une liste de livres ?").as_deref(), Some("collection_create"));
        assert_eq!(k("fais une collection de timbres").as_deref(), Some("collection_create"));
        assert_eq!(k("crée-moi une liste de courses"), None, "les listes à cocher restent aux listes à cocher");
        assert_eq!(k("crée une liste de mails"), None);
        // Ajout
        assert_eq!(k("Ajoute à ma liste vinyle le disque Under a Blood Red Sky du groupe U2.").as_deref(), Some("collection_add"));
        assert_eq!(k("ajoute Thriller de Michael Jackson à ma liste de disques").as_deref(), Some("collection_add"));
        assert_eq!(k("ajoute à ma liste lego le lego 75436 que je veux").as_deref(), Some("collection_add"));
        assert_eq!(k("ajoute du lait à ma liste de courses"), None);
        assert_eq!(k("mets de la musique"), None);
        // Affichage
        assert_eq!(k("affiche moi ma liste de disque").as_deref(), Some("collection_show"));
        assert_eq!(k("montre mes lego").as_deref(), Some("collection_show"));
        assert_eq!(k("affiche ma liste de courses"), None);
        assert_eq!(k("affiche mes mails"), None);
        assert_eq!(k("affiche mes listes").as_deref(), Some("collections_overview"));
        assert_eq!(k("Ouvre moi la carte de liste s'il te plaît.").as_deref(), Some("collections_overview"));
        assert_eq!(k("ouvre-moi le module List.").as_deref(), Some("collections_overview"));
        assert_eq!(k("ouvre la card listes 101").as_deref(), Some("collections_overview"));
        // Retrait et suppression
        assert_eq!(k("retire Thriller de ma liste vinyle").as_deref(), Some("collection_remove_item"));
        assert_eq!(k("supprime ma liste de disques").as_deref(), Some("collection_delete"));
        assert_eq!(k("supprime ma liste de courses"), None);
        // Interroger
        assert_eq!(k("Est-ce que j'ai le lego 75436 ?").as_deref(), Some("collection_have"));
        assert_eq!(k("est-ce que j'ai le disque Thriller").as_deref(), Some("collection_have"));
        assert_eq!(k("j'ai faim"), None);
        assert_eq!(k("j'ai rendez-vous demain"), None);
        assert_eq!(k("combien j'ai de vinyles").as_deref(), Some("collection_count"));
        assert_eq!(k("combien j'ai dépensé en lego").as_deref(), Some("collection_spent"));
        assert_eq!(k("quels disques de U2 j'ai").as_deref(), Some("collection_which"));
        assert_eq!(k("affiche mes souhaits lego").as_deref(), Some("collection_wishes"));
        assert_eq!(k("affiche moi Thriller").as_deref(), Some("collection_open_item"));
        assert_eq!(k("ouvre la fiche du lego 75436").as_deref(), Some("collection_open_item"));
        assert_eq!(k("ouvre Chrome"), None);
        // Schéma, statut, actions
        // Films, séries, concerts
        assert_eq!(k("crée-moi une liste de films").as_deref(), Some("collection_create"));
        assert_eq!(k("fais-moi une liste de séries").as_deref(), Some("collection_create"));
        assert_eq!(k("crée une liste de concerts").as_deref(), Some("collection_create"));
        assert_eq!(k("ajoute à ma liste films le film Inception de Christopher Nolan").as_deref(), Some("collection_add"));
        assert_eq!(k("ajoute Dune à ma liste de films à voir").as_deref(), Some("collection_add"));
        assert_eq!(k("ajoute à ma liste concerts U2 au Stade de France le 12 juin 2024").as_deref(), Some("collection_add"));
        assert_eq!(k("est-ce que j'ai vu Inception").as_deref(), Some("collection_have"));
        assert_eq!(k("j'ai vu Dune").as_deref(), Some("collection_acquired"));
        assert_eq!(k("j'ai vu Inception").as_deref(), Some("collection_have"), "déjà vu : c'est une question");
        assert_eq!(k("affiche mes films à voir").as_deref(), Some("collection_wishes"));
        assert_eq!(k("combien de films j'ai vu").as_deref(), Some("collection_count"));
        assert_eq!(k("combien j'ai dépensé en concerts").as_deref(), Some("collection_spent"));
        assert_eq!(k("ouvre la fiche bande-annonce de Dune"), None, "pas de bouton nommé ainsi : pas d'action");
        assert_eq!(k("ajoute un champ pays à ma liste vinyle").as_deref(), Some("collection_add_field"));
        assert_eq!(k("retire le champ label de ma liste vinyle").as_deref(), Some("collection_remove_field"));
        assert_eq!(k("renomme ma liste vinyle en Mes disques").as_deref(), Some("collection_rename"));
        assert_eq!(k("j'ai acheté Thriller 25 euros à la Fnac").as_deref(), Some("collection_acquired"));
        assert_eq!(k("passe Thriller en possédé").as_deref(), Some("collection_acquired"));
        assert_eq!(k("mets de la musique"), None);
        assert_eq!(k("écoute le vinyle Thriller").as_deref(), Some("collection_action"));
        assert_eq!(k("donne-moi le prix du lego 75436").as_deref(), Some("collection_action"));
        assert_eq!(k("ouvre la fiche lego du 75436").as_deref(), Some("collection_action"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn les_listes_a_cocher_restent_aux_listes_a_cocher() {
        assert_eq!(SmartLists::detect_type("courses pour ce soir"), "courses");
        assert!(schema::detect_preset("disques vinyle").is_some());
        assert!(not_a_list_topic("mes mails"));
    }
}
