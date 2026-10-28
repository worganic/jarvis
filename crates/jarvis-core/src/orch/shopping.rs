// Achat d'un produit (orchestrator/shopping.js) : « je voudrais acheter un aspirateur robot », « trouve-moi
// le meilleur prix pour un casque Sony », « donne-moi le prix de Lego 76934 ». Claude Code part en fond
// (WebSearch / WebFetch), écrit les offres dans workspace/achats/<produit>-<date>.json, puis Jarvis annonce
// le moins cher et affiche une card numérotée (miniature, boutique, prix). ⓘ fiche produit, ☆ bibliothèque
// de produits (data/produits.json). Jarvis ne commande rien : il compare et ouvre la page.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use serde_json::{json, Value};

use super::intent::Intent;
use super::projects::spoken_number;
use super::{OrchRef, Up};
use crate::js::{self, J};

const CACHE_MS: i64 = 30 * 60 * 1000;
const ASK_MS: i64 = 60 * 1000;
const MAX_KNOWN: usize = 400;
const MAX_ITEMS: usize = 200;
const MAX_HISTORY: usize = 30;
const FOOTER: &str = "Prix relevés sur le web par l’IA : à vérifier sur la page du marchand";

const DET: &str = "(?:un |une |des |du |de la |de l |le |la |les |l )?";
const DET_RAW: &str = "(?:un |une |des |du |de la |de l['’]|le |la |les |l['’])?";
const VERBE: &str = "(?:je (?:veux|voudrais|souhaite|souhaiterais) |j aimerais )?";
const DE: &str = "(?:pour|des|du|de|d|sur)";
const DE_RAW: &str = "(?:pour|des|du|de|d['’]|sur)";
const ASK_VERBS: &str = "(?:donne|donner|dis|dire|cherche|chercher|trouve|trouver|retrouve|retrouver|recherche|rechercher|fais|faire)";
const FIND_VERBS: &str = "(?:trouves?|trouver|cherches?|chercher|retrouves?|retrouver|recherches?|rechercher|donnes?|montres?|affiches?)";
const NUM: &str = r"(?:numero |n |no )?(\d{1,2}|un|une|premier|premiere|deux|trois|quatre|cinq|six|sept|huit|neuf|dix)";
const LIB: &str = "(?:ma |la |mes )?(?:bibliotheque|collection|liste)(?: de produits| produits)?";

fn raw_re(prefix: &str) -> regex::Regex {
    regex::Regex::new(&format!(r"(?i){prefix}\s*{DET_RAW}(.+?)[\s.!?]*$")).unwrap()
}

struct Pattern {
    re: regex::Regex,
    raw: regex::Regex,
    model: bool,
}

fn patterns() -> &'static [Pattern] {
    static P: std::sync::LazyLock<Vec<Pattern>> = std::sync::LazyLock::new(|| {
        let r = |s: String| regex::Regex::new(&s).unwrap();
        vec![
            Pattern { re: r(format!("^{VERBE}(?:acheter|achete|achetes)(?: moi)? {DET}(.+)$")), raw: raw_re("(?:acheter|achète|achete|achètes)(?:-moi| moi)?"), model: false },
            Pattern {
                re: r(r"^(?:cherche|chercher|trouve|trouver|recherche|rechercher)(?: moi)? (.+?) (?:a acheter|pas cher|au meilleur prix|moins cher)$".into()),
                raw: r(r"(?i)(?:cherche|chercher|trouve|trouver|recherche|rechercher)(?:-moi| moi)?\s+(.+?)\s+(?:à acheter|a acheter|pas cher|au meilleur prix|moins cher)".into()),
                model: false,
            },
            Pattern {
                re: r(format!("^(?:(?:quel est|c est quoi) |{ASK_VERBS}(?: moi)? )(?:le |les |la )?(?:meilleurs? )?(?:prix|tarifs?)(?: les plus bas)? {DE} {DET}(.+)$")),
                raw: raw_re(&format!(r"(?:prix|tarifs?)(?: les plus bas)?\s+{DE_RAW}")),
                model: false,
            },
            Pattern { re: r(format!("^compare(?: moi)? (?:les )?(?:prix|tarifs?) {DE} {DET}(.+)$")), raw: raw_re(&format!(r"(?:prix|tarifs?)\s+{DE_RAW}")), model: false },
            Pattern {
                re: r(format!("^(?:(?:fais|fait|lance|faire)(?: moi)? (?:une |la )?)?recherche (?:de |des )?prix {DE} {DET}(.+)$")),
                raw: raw_re(&format!(r"recherche\s+(?:de |des )?prix\s+{DE_RAW}")),
                model: false,
            },
            Pattern { re: r(format!("^ou (?:est ce que )?(?:je peux |puis je |on peut )?acheter {DET}(.+)$")), raw: raw_re("acheter"), model: false },
            Pattern { re: r(format!("^combien (?:coute|coutent|ca coute|vaut|valent|il coute) {DET}(.+)$")), raw: raw_re(r"combien\s+(?:coûte|coute|coûtent|coutent|ça coûte|ca coute|vaut|valent)"), model: true },
            Pattern {
                re: r(format!("^(?:je (?:cherche|recherche|veux|voudrais) |{FIND_VERBS}(?: moi)? )(?:le |la |un |une |ce |cet |l )?(?:produit|article) {DET}(.+)$")),
                raw: raw_re("(?:produit|article)"),
                model: false,
            },
            Pattern {
                re: r(format!("^(?:le |ce |mon )?(?:produit|article) (?:est|c est|s appelle|se nomme|recherche est) {DET}(.+)$")),
                raw: raw_re("(?:produit|article)\\s+(?:est|c['’]est|s['’]appelle|se nomme|recherché est)"),
                model: false,
            },
        ]
    });
    &P
}

fn dyn_re(key: &'static str, build: impl FnOnce() -> String) -> &'static regex::Regex {
    use std::sync::{LazyLock, Mutex};
    static CACHE: LazyLock<Mutex<HashMap<&'static str, &'static regex::Regex>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut c = CACHE.lock().unwrap();
    c.entry(key).or_insert_with(|| Box::leak(Box::new(regex::Regex::new(&build()).unwrap())))
}

fn open_re() -> &'static regex::Regex {
    dyn_re("open", || format!("^(?:ouvre|ouvrir|affiche|montre|va sur|regarde)(?: moi)? (?:le |l |ce )?(?:produit|article|offre|resultat) {NUM}$"))
}
fn detail_re() -> &'static regex::Regex {
    dyn_re("detail", || format!("^(?:(?:ouvre|affiche|montre|donne)(?: moi)? )?(?:la |le |les )?(?:fiche|details?|description|infos?|informations?|caracteristiques)(?: du| de l| de la)? ?(?:produit|article|offre) {NUM}$"))
}
fn keep_re() -> &'static regex::Regex {
    dyn_re("keep", || format!("^(?:garde|gardes|conserve|sauvegarde|enregistre|ajoute|memorise)(?: moi)? (?:le |l |ce |cet )?(?:produit|article|offre)(?: {NUM})?(?: (?:dans|a|en|sur) {LIB})?$"))
}
fn library_re() -> &'static regex::Regex {
    dyn_re("library", || "^(?:(?:ouvre|affiche|montre|liste|donne)(?: moi)? (?:ma |la |mes |les )?(?:bibliotheque(?: de produits| produits)?|produits? (?:gardes?|sauvegardes?|enregistres?|en memoire)|produits)|mes produits|ma bibliotheque(?: de produits)?|bibliotheque de produits)$".into())
}
fn remove_re() -> &'static regex::Regex {
    dyn_re("remove", || format!("^(?:supprime|retire|enleve|efface|vire)(?: moi)? (?:le |l |ce |cet )?(?:produit|article) {NUM}(?: (?:de|dans|du) {LIB})?$"))
}
fn remove_lib_re() -> &'static regex::Regex {
    dyn_re("remove_lib", || format!("(?:de|dans|du) {LIB}$"))
}
fn refresh_re() -> &'static regex::Regex {
    dyn_re("refresh", || format!("^(?:actualise|actualiser|mets a jour|met a jour|rafraichis|refais|relance)(?: moi)? (?:la recherche |les prix |le prix )?(?:du |de l |pour le |pour l )?(?:produit|article) {NUM}$"))
}

fn library_words(t: &str) -> bool {
    crate::re!(r"\bbibliotheques?\b.*\bproduits?\b|\bproduits?\b.*\bbibliotheques?\b").is_match(t)
        || crate::re!(r"\b(?:mes|nos) produits\b").is_match(t)
        || crate::re!(r"\bliste (?:de |des )?(?:mes )?produits\b").is_match(t)
        || crate::re!(r"\bproduits? (?:que j ai |que tu as )?(?:gardes?|sauvegardes?|enregistres?|memorises?|conserves?|en memoire)\b").is_match(t)
}

fn no_product(t: &str) -> bool {
    crate::re!(r"\b(?:retrouv|trouv|cherch|recherch|fai|lanc)\w*\b.*\b(?:produit|article)\b.*\bprix\b").is_match(t)
        || crate::re!(r"\bprix\b.*\b(?:d un|d une|du) (?:produit|article)\b").is_match(t)
        || crate::re!(r"^(?:(?:fais|fait|lance|faire)(?: moi)? (?:une |la )?)?recherche (?:de |des )?prix$").is_match(t)
        || crate::re!(r"^(?:cherche|chercher|trouve|trouver|retrouve|retrouver)(?: moi)? (?:un |le |ce )?(?:produit|article)$").is_match(t)
}

fn not_a_name(s: &str) -> bool {
    crate::re!(r"^(?:pour|de|du|des|d|qui|que|qu|dans|avec|sur|en|au|aux|a|et|ou|mais|stp|svp|s il|merci|moins|plus|pas|si|quand|comme)\b").is_match(s)
}

pub fn clean(v: &J, max: usize) -> String {
    let s = if v.is_null() { String::new() } else { js::s(v) };
    clean_str(&s, max)
}

fn clean_str(s: &str, max: usize) -> String {
    let s = crate::re!(r"[\x00-\x1f\x7f]+").replace_all(s, " ");
    js::slice_chars(crate::re!(r"\s+").replace_all(&s, " ").trim(), max)
}

fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

fn slug(s: &str) -> String {
    let t = js::slice_chars(&fold(s).replace(' ', "-"), 50);
    if t.is_empty() { "produit".into() } else { t }
}

fn hash(s: &str) -> String {
    let h = sha1_smol::Sha1::from(s).hexdigest();
    h[..10].to_string()
}

fn short_date(ms: i64) -> String {
    js::local_of(ms).format("%d/%m").to_string()
}

fn call(id: &str, action: &str) -> J {
    json!({ "method": "listAction", "args": ["produit", id, action] })
}

/// 129.9 → « 129,90 € »
pub fn fmt_price(n: f64) -> String {
    let s = format!("{n:.2}").replace('.', ",");
    format!("{} €", s.strip_suffix(",00").unwrap_or(&s))
}

/// 129.9 → « 129 euros 90 » (lu à voix haute)
pub fn spoken_price(n: f64) -> String {
    let euros = n.floor() as i64;
    let cents = ((n - euros as f64) * 100.0).round() as i64;
    format!("{euros} euro{}{}", if euros > 1 { "s" } else { "" }, if cents != 0 { format!(" {cents}") } else { String::new() })
}

pub fn parse_price(v: &J) -> Option<f64> {
    match v {
        Value::Number(n) => {
            let f = n.as_f64()?;
            (f.is_finite() && f > 0.0 && f < 1e6).then(|| (f * 100.0).round() / 100.0)
        }
        Value::String(s) => {
            let s = crate::re!(r"\s|&nbsp;").replace_all(s, "");
            let s = crate::re!(r"(?i)eur(?:os?)?|€").replace_all(&s, "");
            let s = crate::fre!(r"\.(?=\d{3}(?:,|$))").replace_all(&s, "").into_owned();
            let m = crate::re!(r"^(\d{1,6})(?:[.,](\d{1,2}))?$").captures(&s)?;
            let f: f64 = format!("{}.{}", &m[1], m.get(2).map(|x| x.as_str()).unwrap_or("0")).parse().ok()?;
            parse_price(&json!(f))
        }
        _ => None,
    }
}

/// URL http(s) sans identifiants, sans fragment ; None sinon.
pub fn safe_url(v: &J) -> Option<String> {
    let mut u = reqwest::Url::parse(js::s(v).trim()).ok()?;
    if u.scheme() != "https" && u.scheme() != "http" {
        return None;
    }
    if !u.username().is_empty() || u.password().is_some() || !u.host_str().map(|h| h.contains('.')).unwrap_or(false) {
        return None;
    }
    u.set_fragment(None);
    let s = u.to_string();
    if s.chars().count() <= 500 {
        return Some(s);
    }
    // Adresse trop longue (pistage d'une boutique : ?crid=…&dib=…) : sans la requête, elle ouvre le plus souvent le même produit.
    u.set_query(None);
    let short = u.to_string();
    (short.chars().count() <= 500).then_some(short)
}

pub fn safe_image(v: &J) -> String {
    safe_url(v).filter(|u| u.starts_with("https://")).unwrap_or_default()
}

fn shipping_label(v: &J) -> String {
    if let Some(n) = v.as_f64() {
        return if n == 0.0 {
            "livraison offerte".into()
        } else if n > 0.0 && n < 1000.0 {
            format!("+ {} de livraison", fmt_price(n))
        } else {
            String::new()
        };
    }
    let s = clean(v, 40);
    if crate::re!(r"(?i)^(?:null|n/a|inconnue?|\?)$").is_match(&s) { String::new() } else { s }
}

/// Offres du JSON de Claude → liste propre, du moins cher au plus cher (sans prix en dernier).
pub fn normalize_offers(data: &J, max: f64) -> Vec<J> {
    let list = if data.is_array() { js::arr(data) } else { js::arr(&data["offers"]) };
    let mut seen = HashSet::new();
    let mut out: Vec<J> = vec![];
    for o in list.iter().filter(|o| o.is_object()) {
        let Some(url) = safe_url(&o["url"]) else { continue };
        let name = clean(if js::truthy(&o["name"]) { &o["name"] } else { &o["title"] }, 120);
        if name.is_empty() || seen.contains(&url) {
            continue;
        }
        seen.insert(url.clone());
        let mut store = clean(if js::truthy(&o["store"]) { &o["store"] } else { &o["shop"] }, 40);
        if store.is_empty() {
            store = reqwest::Url::parse(&url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())).unwrap_or_default();
        }
        let mut offer = json!({
            "id": hash(&url), "name": name, "store": store, "url": url,
            "price": parse_price(&o["price"]), "shipping": shipping_label(&o["shipping"]),
            "note": clean(&o["note"], 80), "image": safe_image(&o["image"])
        });
        if let Some(b) = o["inStock"].as_bool() {
            offer["inStock"] = json!(b);
        }
        out.push(offer);
    }
    out.sort_by(|a, b| {
        let (pa, pb) = (a["price"].as_f64(), b["price"].as_f64());
        pa.is_none().cmp(&pb.is_none()).then(pa.unwrap_or(0.0).partial_cmp(&pb.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
    });
    let max = if max.is_finite() && max != 0.0 { max } else { 10.0 };
    out.truncate(max.clamp(3.0, 15.0) as usize);
    out
}

/// Produit précis identifié par l'IA (modèle ou référence).
pub fn normalize_product(p: &J) -> Option<J> {
    if !p.is_object() {
        return None;
    }
    let name = clean(&p["name"], 120);
    if name.is_empty() {
        return None;
    }
    let specs: Vec<J> = js::arr(&p["specs"])
        .iter()
        .map(|s| json!({ "label": clean(&s["label"], 30), "value": clean(&s["value"], 60) }))
        .filter(|s| js::truthy(&s["label"]) && js::truthy(&s["value"]))
        .take(8)
        .collect();
    Some(json!({ "name": name, "brand": clean(&p["brand"], 40), "reference": clean(&p["reference"], 40), "description": clean(&p["description"], 300), "image": safe_image(&p["image"]), "specs": specs }))
}

/// JSON écrit par Claude dans le fichier ; à défaut, bloc JSON de sa réponse.
pub fn read_offers_data(file: &std::path::Path, answer: &str) -> Option<J> {
    if let Some(d) = std::fs::read_to_string(file).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()) {
        return Some(d);
    }
    let fenced = crate::re!(r"```(?:json)?\s*([\s\S]+?)```").captures(answer).map(|m| m[1].to_string());
    let braces = match (answer.find('{'), answer.rfind('}')) {
        (Some(a), Some(b)) if b > a => Some(answer[a..=b].to_string()),
        _ => None,
    };
    for chunk in [fenced, braces].into_iter().flatten() {
        if let Ok(v) = serde_json::from_str(&chunk) {
            return Some(v);
        }
    }
    None
}

fn best_of(offers: &J) -> Option<J> {
    js::arr(offers).into_iter().find(|o| o["price"].is_number())
}

fn price_of(o: &J) -> f64 {
    o["price"].as_f64().unwrap_or(0.0)
}

struct Pending {
    query: String,
    file: PathBuf,
    key: String,
    refresh_id: Option<String>,
}

#[derive(Clone)]
enum Shown {
    Search(J),
    Library(Vec<String>),
}

pub struct Shopping {
    o: OrchRef,
    file: Option<PathBuf>,
    log: std::rc::Rc<dyn Fn(String)>,
    pending: RefCell<HashMap<String, Pending>>,
    /// id d'offre → (offre, recherche), ordre d'insertion pour la limite
    known: RefCell<(HashMap<String, (J, J)>, VecDeque<String>)>,
    cache: RefCell<HashMap<String, (i64, J)>>,
    shown: RefCell<Option<Shown>>,
    awaiting: RefCell<Option<i64>>,
    pub items: RefCell<Vec<J>>,
}

enum Numbered {
    Offer(J, J),
    Item(J),
    Message(J),
}

impl Shopping {
    pub fn new(o: OrchRef, file: Option<PathBuf>, log: std::rc::Rc<dyn Fn(String)>) -> Self {
        let items = file
            .as_ref()
            .and_then(|f| js::read_json(f))
            .and_then(|d| d.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|i| js::truthy(&i["id"]) && js::truthy(&i["name"]))
            .take(MAX_ITEMS)
            .collect();
        Self {
            o,
            file,
            log,
            pending: RefCell::new(HashMap::new()),
            known: RefCell::new((HashMap::new(), VecDeque::new())),
            cache: RefCell::new(HashMap::new()),
            shown: RefCell::new(None),
            awaiting: RefCell::new(None),
            items: RefCell::new(items),
        }
    }

    fn cfg(&self) -> (f64, String) {
        let c = self.o.up().config.get("shopping");
        let max = js::num(&c["maxOffers"]).filter(|n| *n != 0.0).unwrap_or(10.0);
        (max, clean(&c["stores"], 200))
    }

    fn dir(&self) -> PathBuf {
        crate::paths::workspace().join("achats")
    }

    // --- Bibliothèque de produits ---------------------------------------------------------------------

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(d) = file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let text = serde_json::to_string_pretty(&Value::Array(self.items.borrow().clone())).unwrap_or_default();
        if let Err(e) = std::fs::write(file, text) {
            (self.log)(format!("bibliothèque de produits : écriture impossible ({e})"));
        }
    }

    fn sorted(&self) -> Vec<J> {
        let mut l = self.items.borrow().clone();
        l.sort_by(|a, b| js::gn_or(b, "updatedAt", 0.0).partial_cmp(&js::gn_or(a, "updatedAt", 0.0)).unwrap_or(std::cmp::Ordering::Equal));
        l
    }

    fn find_item(&self, id: &str) -> Option<J> {
        self.items.borrow().iter().find(|i| js::gs(i, "id") == id).cloned()
    }

    fn item_from(search: &J, offer: &J) -> J {
        let p = &search["product"];
        let best = best_of(&search["offers"]);
        let mut base = if p.is_object() {
            let image = if js::truthy(&p["image"]) { js::gs(p, "image") } else { best.as_ref().map(|b| js::gs(b, "image")).unwrap_or_default() };
            json!({ "name": p["name"], "brand": p["brand"], "reference": p["reference"], "description": p["description"], "image": image, "specs": p["specs"], "offers": search["offers"] })
        } else {
            json!({ "name": offer["name"], "brand": "", "reference": "", "description": js::gs(offer, "note"), "image": js::gs(offer, "image"), "specs": [], "offers": [offer] })
        };
        let offers: Vec<J> = js::arr(&base["offers"]).into_iter().take(10).collect();
        base["offers"] = json!(offers);
        let mut out = json!({ "id": format!("p-{}", hash(&fold(&js::gs(&base, "name")))), "query": search["query"] });
        js::assign(&mut out, base);
        out
    }

    /// Met à jour (ou crée) un produit ; l'historique garde chaque nouveau meilleur prix.
    fn upsert(&self, data: &J, now: i64) -> (J, bool, Option<J>, Option<J>) {
        let id = js::gs(data, "id");
        let mut items = self.items.borrow_mut();
        let created = !items.iter().any(|i| js::gs(i, "id") == id);
        if created {
            items.insert(0, json!({ "id": id, "savedAt": now, "history": [] }));
            items.truncate(MAX_ITEMS);
        }
        let item = items.iter_mut().find(|i| js::gs(i, "id") == id).unwrap();
        let before = if created { None } else { best_of(&item["offers"]) };
        let pick = |k: &str| if js::truthy(&data[k]) { data[k].clone() } else if js::truthy(&item[k]) { item[k].clone() } else { json!("") };
        let specs = if js::arr(&data["specs"]).is_empty() { if item["specs"].is_array() { item["specs"].clone() } else { json!([]) } } else { data["specs"].clone() };
        let query = if js::truthy(&data["query"]) { data["query"].clone() } else if js::truthy(&item["query"]) { item["query"].clone() } else { data["name"].clone() };
        let patch = json!({
            "name": data["name"], "brand": pick("brand"), "reference": pick("reference"), "description": pick("description"), "image": pick("image"),
            "specs": specs, "query": query, "offers": data["offers"], "updatedAt": now
        });
        js::assign(item, patch);
        let best = best_of(&item["offers"]);
        if !item["history"].is_array() {
            item["history"] = json!([]);
        }
        if let Some(b) = &best {
            let hist = item["history"].as_array_mut().unwrap();
            if hist.last().map(|h| h["price"] != b["price"]).unwrap_or(true) {
                hist.push(json!({ "at": now, "price": b["price"], "store": b["store"] }));
                if hist.len() > MAX_HISTORY {
                    hist.remove(0);
                }
            }
        }
        let out = item.clone();
        drop(items);
        self.save();
        (out, created, before, best)
    }

    // --- Intentions du routeur ---------------------------------------------------------------------

    fn is_generic(query: &str) -> bool {
        query.chars().count() < 3 || crate::re!(r"^(?:(?:le |la |un |une |ce |cet |cette |des |les |mes |mon |ma |nos )?(?:produit|produits|article|articles|truc|machin|objet|chose|quelque chose)|ca|cela|ceci|rien)$").is_match(&fold(query))
    }

    fn intent<F, Fu>(&self, kind: &str, label: &str, detail: &str, f: F) -> Intent
    where
        F: Fn(&Shopping) -> Fu + 'static,
        Fu: std::future::Future<Output = J> + 'static,
    {
        let _ = &f;
        let o = self.o.clone();
        let f = std::rc::Rc::new(f);
        let it = Intent::new(kind, label, detail, move || {
            let o = o.clone();
            let f = f.clone();
            async move {
                let orch = o.up();
                f(&orch.shopping).await
            }
        });
        if kind == "shopping_search" { it.free_text() } else { it }
    }

    /// Pure : rien ne change avant execute().
    pub fn match_(&self, t: &str, raw: &str) -> Option<Intent> {
        let number = |s: &str| spoken_number(s).unwrap_or(0);
        if let Some(m) = open_re().captures(t) {
            let n = number(&m[1]);
            return Some(self.intent("shopping_open", "Produit", &format!("n° {n}"), move |s| s.open(n)));
        }
        if crate::re!(r"^(?:ouvre|ouvrir|affiche|montre|va sur)(?: moi)? (?:la |l )?(?:meilleure offre|meilleur prix|offre la moins chere)$|^(?:ouvre|ouvrir|affiche|montre|va sur)(?: moi)? (?:le )?(?:moins cher|produit le moins cher)$").is_match(t) {
            return Some(self.intent("shopping_open", "Produit", "le moins cher", |s| s.open(1)));
        }
        if let Some(m) = detail_re().captures(t) {
            let n = number(&m[1]);
            return Some(self.intent("shopping_detail", "Fiche produit", &format!("n° {n}"), move |s| {
                let r = s.detail(n);
                async move { r }
            }));
        }
        if let Some(m) = remove_re().captures(t) {
            let n = number(&m[1]);
            let lib = remove_lib_re().is_match(t);
            return Some(self.intent("shopping_remove", "Bibliothèque", &format!("retirer le n° {n}"), move |s| {
                let r = s.remove_number(n, lib);
                async move { r }
            }));
        }
        if let Some(m) = refresh_re().captures(t) {
            let n = number(&m[1]);
            return Some(self.intent("shopping_refresh", "Produit", &format!("actualiser le n° {n}"), move |s| {
                let r = s.refresh_number(n);
                async move { r }
            }));
        }
        if let Some(m) = keep_re().captures(t) {
            let n = m.get(1).map(|x| number(x.as_str())).unwrap_or(1);
            return Some(self.intent("shopping_keep", "Bibliothèque", &format!("garder le n° {n}"), move |s| {
                let r = s.keep_number(n);
                async move { r }
            }));
        }
        let words = t.split(' ').count();
        if library_re().is_match(t) || (words <= 16 && !crate::re!(r"\b(?:achet|prix|tarif)\w*").is_match(t) && library_words(t)) {
            return Some(self.intent("shopping_library", "Bibliothèque", "produits gardés", |s| {
                let r = s.library_card();
                async move { r }
            }));
        }
        for p in patterns() {
            let Some(m) = p.re.captures(t) else { continue };
            let query = p.raw.captures(raw).and_then(|c| c.get(1).map(|x| clean_str(x.as_str(), 100))).filter(|q| !q.is_empty()).unwrap_or_else(|| m[1].trim().to_string());
            let rest: String = query.chars().skip(1).collect();
            if p.model && !crate::re!(r"\d|[A-ZÀ-Ý]").is_match(&rest) {
                return None;
            }
            if Self::is_generic(&query) {
                return Some(self.intent("shopping_ask", "Recherche de produit", "quel produit ?", |s| {
                    let r = s.ask();
                    async move { r }
                }));
            }
            let q = query.clone();
            return Some(self.intent("shopping_search", "Recherche de produit", &query, move |s| {
                let r = s.search(&q, None);
                async move { r }
            }));
        }
        if words <= 20 && crate::re!(r"\b(?:trouv|cherch|retrouv|recherch|veu|voudr|aimer|prix|achet|info|tarif|compar)\w*").is_match(t) {
            if let Some(loose) = crate::re!(r"\b(?:produit|article)s? (?:est |c est )?(?:un |une |le |la |l |des )?(.+)$").captures(t) {
                if !not_a_name(&loose[1]) {
                    let from_raw = crate::re!(r"(?i)\b(?:produit|article)s?\s+(?:est\s+|c['’]est\s+)?(?:un |une |le |la |l['’]|des )?(.+?)[\s.!?]*$").captures(raw).map(|c| c[1].to_string());
                    let query = clean_str(&from_raw.unwrap_or_else(|| loose[1].to_string()), 100);
                    if !Self::is_generic(&query) && !not_a_name(&fold(&query)) {
                        let q = query.clone();
                        return Some(self.intent("shopping_search", "Recherche de produit", &query, move |s| {
                            let r = s.search(&q, None);
                            async move { r }
                        }));
                    }
                }
            }
        }
        if words <= 16 && no_product(t) {
            return Some(self.intent("shopping_ask", "Recherche de produit", "quel produit ?", |s| {
                let r = s.ask();
                async move { r }
            }));
        }
        let waiting = self.awaiting.borrow().map(|at| js::now_ms() - at < ASK_MS).unwrap_or(false);
        if waiting {
            return self.follow_up(t, raw);
        }
        None
    }

    /// Réponse à « quel produit ? » : le produit, ou l'abandon.
    fn follow_up(&self, t: &str, raw: &str) -> Option<Intent> {
        if crate::re!(r"^(?:non|annule|annuler|laisse tomber|oublie|rien|tant pis|finalement non)\b").is_match(t) {
            return Some(self.intent("shopping_cancel", "Recherche de produit", "annulée", |s| {
                *s.awaiting.borrow_mut() = None;
                async { json!({ "say": "D’accord, j’annule la recherche.", "display": "Recherche annulée" }) }
            }));
        }
        if t.chars().count() < 2
            || t.split(' ').count() > 12
            || crate::re!(r"^(?:quel|quelle|quels|quelles|combien|comment|pourquoi|qui|quand|ou|est ce|est il|peux|pourrais|tu|dis|lis|ouvre|ferme|lance|mets|baisse|monte|arrete|stop|merci|ok|oui|jarvis)\b").is_match(t)
        {
            return None;
        }
        let mut query = clean_str(raw, 100);
        loop {
            let next = crate::re!(r"(?i)^(?:jarvis[\s,]+|c['’]est (?:le |la |un |une )?|il s['’]agit (?:de |du |d['’])|je cherche |je veux |le |la |les |un |une |l['’])").replace(&query, "").into_owned();
            if next == query {
                break;
            }
            query = next;
        }
        let query = crate::re!(r"[\s.!?]+$").replace(&query, "").into_owned();
        if Self::is_generic(&query) {
            return None;
        }
        let q = query.clone();
        Some(self.intent("shopping_search", "Recherche de produit", &query, move |s| {
            let r = s.search(&q, None);
            async move { r }
        }))
    }

    fn ask(&self) -> J {
        *self.awaiting.borrow_mut() = Some(js::now_ms());
        json!({ "say": "Quel produit cherches-tu ? Dis-moi le nom, la marque ou le modèle exact.", "display": "Quel produit ?" })
    }

    // --- Recherche en fond ---------------------------------------------------------------------------

    fn prompt(&self, query: &str, file: &str) -> String {
        let (_, stores) = self.cfg();
        [
            format!("Recherche d'achat : « {query} »."),
            "Utilise la recherche web (WebSearch puis WebFetch) pour trouver où acheter ce produit en France :".into(),
            format!("au moins 5 offres réelles, d'au moins 3 boutiques fiables différentes{}, avec des produits qui correspondent vraiment à la demande.", if stores.is_empty() { String::new() } else { format!(" (de préférence : {stores})") }),
            "Prix TTC en euros tels qu'affichés sur la page ; ajoute les frais de livraison s'ils sont indiqués.".into(),
            "Ne devine ni n'invente jamais un prix, une adresse ni une image : price vaut null et image vaut \"\" si tu ne les as pas vus ; chaque url est une page que tu as réellement consultée.".into(),
            "image : adresse https d'une photo du produit lue sur la page (image principale ou og:image).".into(),
            "product : remplis-le seulement si la demande désigne un produit précis (modèle, référence) ; sinon null.".into(),
            format!("Écris UNIQUEMENT du JSON valide (sans Markdown) dans le fichier {file} (crée le dossier au besoin) :"),
            "{\"query\": \"…\", \"product\": {\"name\": \"nom exact\", \"brand\": \"marque\", \"reference\": \"référence\", \"description\": \"une ou deux phrases, 200 caractères au plus\", \"image\": \"https://…\", \"specs\": [{\"label\": \"Pièces\", \"value\": \"3643\"}]},".into(),
            "\"offers\": [{\"name\": \"nom exact du produit\", \"store\": \"boutique\", \"price\": 129.9, \"shipping\": \"livraison offerte\", \"url\": \"https://…\", \"image\": \"https://…\", \"inStock\": true, \"note\": \"détail utile, 80 caractères au plus\"}]}".into(),
            "Six caractéristiques au plus. Les offres sont triées du prix le plus bas au plus haut.".into(),
            "Ta réponse finale est lue à voix haute : deux phrases au plus, sans markdown, le meilleur prix trouvé et sa boutique.".into(),
        ]
        .join(" ")
    }

    pub fn search(&self, query: &str, refresh_id: Option<String>) -> J {
        *self.awaiting.borrow_mut() = None;
        let key = fold(query);
        let hit = self.cache.borrow().get(&key).cloned();
        if refresh_id.is_none() {
            if let Some((at, search)) = hit {
                if js::now_ms() - at < CACHE_MS && !js::arr(&search["offers"]).is_empty() {
                    self.show_search(&search);
                    let q = js::gs(&search, "query");
                    return json!({ "say": format!("J'ai déjà les prix pour {q}, je te les réaffiche. {}", Self::best_sentence(&search["offers"])), "display": format!("Achat : {q}"), "quiet": true });
                }
            }
        }
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M").to_string();
        let file = self.dir().join(format!("{}-{stamp}.json", slug(query)));
        let text = self.prompt(query, &file.to_string_lossy());
        let refreshing = refresh_id.is_some();
        self.pending.borrow_mut().insert(text.clone(), Pending { query: query.to_string(), file, key, refresh_id });
        self.o.up().enqueue_ai(text, json!({ "background": true }));
        json!({ "say": format!("Je cherche {} {query}. Je te donne les prix dans un instant.", if refreshing { "les prix à jour de" } else { "où acheter" }), "display": format!("Achat : {query}"), "quiet": true })
    }

    fn best_sentence(offers: &J) -> String {
        match best_of(offers) {
            Some(b) => format!("Le moins cher : {} chez {}.", spoken_price(price_of(&b)), js::gs(&b, "store")),
            None => {
                let n = js::arr(offers).len();
                format!("{n} offre{}, sans prix confirmé : ouvre-les pour vérifier.", if n > 1 { "s" } else { "" })
            }
        }
    }

    /// Fin de la tâche de fond : true si c'était une recherche d'achat.
    pub fn done(&self, text: &str, answer: &str, ok: bool) -> bool {
        let Some(r) = self.pending.borrow_mut().remove(text) else { return false };
        let o = self.o.up();
        let notify = |m: String| o.notify(json!({ "text": m, "source": "achat" }));
        if !ok {
            notify(format!("La recherche de prix pour {} n'a pas abouti.", r.query));
            return true;
        }
        let data = read_offers_data(&r.file, answer).unwrap_or(Value::Null);
        let offers = normalize_offers(&data, self.cfg().0);
        if offers.is_empty() {
            notify(format!("Je n'ai pas trouvé d'offre fiable pour {}. Précise le modèle ou la marque, et je réessaie.", r.query));
            return true;
        }
        let now = js::now_ms();
        let product = normalize_product(&data["product"]).unwrap_or(Value::Null);
        let search = json!({ "query": r.query, "product": product, "offers": offers, "at": now });
        self.cache.borrow_mut().insert(r.key.clone(), (now, search.clone()));
        let mut change = String::new();
        if let Some(rid) = &r.refresh_id {
            if let Some(item) = self.find_item(rid) {
                let mut data = item.clone();
                if product.is_object() {
                    js::assign(&mut data, product.clone());
                }
                data["image"] = if js::truthy(&product["image"]) { product["image"].clone() } else { item["image"].clone() };
                data["offers"] = json!(offers);
                let (_, _, before, after) = self.upsert(&data, now);
                if let (Some(b), Some(a)) = (before, after) {
                    let (pb, pa) = (price_of(&b), price_of(&a));
                    change = if pb != pa {
                        format!(" Avant : {}, maintenant {}{}.", spoken_price(pb), spoken_price(pa), if pa < pb { " : ça a baissé" } else { " : ça a augmenté" })
                    } else {
                        " Le prix n’a pas changé.".into()
                    };
                }
            }
        }
        let n = offers.len();
        notify(format!("{n} offre{} pour {}. {}{change}", if n > 1 { "s" } else { "" }, r.query, Self::best_sentence(&search["offers"])));
        self.show_search(&search);
        true
    }

    // --- Cards -----------------------------------------------------------------------------------------

    fn search_rows(&self, search: &J) -> Vec<J> {
        let fallback = js::gs(&search["product"], "image");
        js::arr(&search["offers"])
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let id = js::gs(o, "id");
                let thumb = if js::truthy(&o["image"]) { js::gs(o, "image") } else { fallback.clone() };
                let sub: Vec<String> = [js::gs(o, "store"), js::gs(o, "shipping"), if o["inStock"] == json!(false) { "indisponible".into() } else { String::new() }, js::gs(o, "note")]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                let mut row = json!({
                    "icon": "🛒",
                    "title": format!("{}. {}", i + 1, js::gs(o, "name")),
                    "sub": sub.join(" · "),
                    "meta": o["price"].as_f64().map(fmt_price).unwrap_or_else(|| "prix ?".into()),
                    "say": format!("Ouvre le produit {}", i + 1),
                    "call": call(&id, "ouvrir"),
                    "hint": "Clic : ouvrir la page du marchand",
                    "actions": [
                        { "icon": "ⓘ", "title": "Fiche du produit", "say": format!("Fiche du produit {}", i + 1), "call": call(&id, "fiche") },
                        { "icon": if self.is_kept(search, o) { "★" } else { "☆" }, "title": "Garder dans ma bibliothèque de produits", "say": format!("Garde le produit {}", i + 1), "call": call(&id, "garder") }
                    ]
                });
                if !thumb.is_empty() {
                    row["thumb"] = json!(thumb);
                }
                if i == 0 && o["price"].is_number() {
                    row["tone"] = json!("ok");
                }
                row
            })
            .collect()
    }

    fn is_kept(&self, search: &J, offer: &J) -> bool {
        let id = js::gs(&Self::item_from(search, offer), "id");
        self.items.borrow().iter().any(|i| js::gs(i, "id") == id)
    }

    fn show_search(&self, search: &J) {
        *self.shown.borrow_mut() = Some(Shown::Search(search.clone()));
        {
            let mut known = self.known.borrow_mut();
            for o in js::arr(&search["offers"]) {
                let id = js::gs(&o, "id");
                if known.0.insert(id.clone(), (o, search.clone())).is_none() {
                    known.1.push_back(id);
                }
            }
            while known.0.len() > MAX_KNOWN {
                let Some(old) = known.1.pop_front() else { break };
                known.0.remove(&old);
            }
        }
        if search["product"].is_object() {
            let offer = best_of(&search["offers"]).or_else(|| js::arr(&search["offers"]).into_iter().next()).unwrap_or(Value::Null);
            self.show_detail(Some(search), Some(&offer), None);
            return;
        }
        let q = js::gs(search, "query");
        self.o.up().show_widget(
            "tool",
            json!({ "title": format!("Achat · {q}"), "tool": "achat", "rows": self.search_rows(search), "count": js::arr(&search["offers"]).len(), "ttl": 900000, "wide": true, "footer": FOOTER }),
        );
    }

    fn library_card(&self) -> J {
        let items = self.sorted();
        *self.shown.borrow_mut() = Some(Shown::Library(items.iter().map(|i| js::gs(i, "id")).collect()));
        let rows: Vec<J> = items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let best = best_of(&it["offers"]);
                let hist = js::arr(&it["history"]);
                let prev = if hist.len() > 1 { hist[hist.len() - 2]["price"].as_f64() } else { None };
                let trend = match (&best, prev) {
                    (Some(b), Some(p)) if p != price_of(b) => {
                        if price_of(b) < p { " ▼" } else { " ▲" }
                    }
                    _ => "",
                };
                let at = js::num(&it["updatedAt"]).filter(|x| *x != 0.0).or_else(|| js::num(&it["savedAt"])).unwrap_or(0.0) as i64;
                let sub: Vec<String> = [best.as_ref().map(|b| js::gs(b, "store")).unwrap_or_else(|| "sans prix".into()), format!("relevé le {}", short_date(at)), js::gs(it, "brand")]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                let id = js::gs(it, "id");
                let mut row = json!({
                    "icon": "🛒",
                    "title": format!("{}. {}", i + 1, js::gs(it, "name")),
                    "sub": sub.join(" · "),
                    "meta": best.as_ref().map(|b| format!("{}{trend}", fmt_price(price_of(b)))).unwrap_or_else(|| "prix ?".into()),
                    "say": format!("Ouvre le produit {}", i + 1),
                    "call": call(&id, "ouvrir"),
                    "hint": "Clic : ouvrir la meilleure offre",
                    "actions": [
                        { "icon": "ⓘ", "title": "Fiche du produit", "say": format!("Fiche du produit {}", i + 1), "call": call(&id, "fiche") },
                        { "icon": "↻", "title": "Actualiser les prix", "say": format!("Actualise les prix du produit {}", i + 1), "call": call(&id, "actualiser") },
                        { "icon": "🗑", "title": "Retirer de la bibliothèque", "tone": "danger", "say": format!("Supprime le produit {} de ma bibliothèque", i + 1), "call": call(&id, "supprimer"), "done": "retiré" }
                    ]
                });
                if js::truthy(&it["image"]) {
                    row["thumb"] = it["image"].clone();
                }
                row
            })
            .collect();
        let n = rows.len();
        self.o.up().show_widget(
            "tool",
            json!({
                "title": "Bibliothèque de produits", "tool": "produits", "rows": rows, "count": n, "ttl": 900000, "wide": true,
                "empty": "Aucun produit gardé : après une recherche, dis « garde le produit 2 ».",
                "footer": if n > 0 { FOOTER } else { "" }
            }),
        );
        json!({
            "say": if n > 0 { format!("Ta bibliothèque compte {n} produit{}.", if n > 1 { "s" } else { "" }) } else { "Ta bibliothèque de produits est vide. Après une recherche, dis « garde le produit 2 ».".into() },
            "display": "Bibliothèque de produits", "quiet": true
        })
    }

    /// Fiche du produit : card dédiée (image, description, caractéristiques, toutes les offres).
    fn show_detail(&self, search: Option<&J>, offer: Option<&J>, item: Option<&J>) -> J {
        let null = Value::Null;
        let offer = offer.unwrap_or(&null);
        let src = if let Some(item) = item {
            json!({ "name": item["name"], "brand": item["brand"], "reference": item["reference"], "description": item["description"], "image": item["image"], "specs": if item["specs"].is_array() { item["specs"].clone() } else { json!([]) }, "offers": if item["offers"].is_array() { item["offers"].clone() } else { json!([]) } })
        } else if let Some(p) = search.map(|s| &s["product"]).filter(|p| p.is_object()) {
            let mut s = p.clone();
            s["offers"] = search.unwrap()["offers"].clone();
            s["image"] = if js::truthy(&p["image"]) { p["image"].clone() } else { json!(js::gs(offer, "image")) };
            s
        } else {
            json!({ "name": offer["name"], "brand": "", "reference": "", "description": js::gs(offer, "note"), "image": js::gs(offer, "image"), "specs": [], "offers": [offer] })
        };
        let kept = match item {
            Some(i) => Some(i.clone()),
            None => search.and_then(|s| self.find_item(&js::gs(&Self::item_from(s, offer), "id"))),
        };
        let offers = js::arr(&src["offers"]);
        let best = best_of(&src["offers"]);
        let best_id = best.as_ref().map(|b| js::gs(b, "id"));
        let ref_id = match &kept {
            Some(k) => js::gs(k, "id"),
            None => {
                if offer.is_object() {
                    js::gs(offer, "id")
                } else if let Some(b) = &best {
                    js::gs(b, "id")
                } else {
                    offers.first().map(|o| js::gs(o, "id")).unwrap_or_default()
                }
            }
        };
        let kept_at = kept.as_ref().map(|k| js::num(&k["updatedAt"]).filter(|x| *x != 0.0).or_else(|| js::num(&k["savedAt"])).unwrap_or(0.0) as i64);
        let history: Vec<J> = match &kept {
            Some(k) if js::arr(&k["history"]).len() > 1 => {
                let h = js::arr(&k["history"]);
                h[h.len().saturating_sub(5)..].iter().map(|h| json!({ "at": short_date(js::gn_or(h, "at", 0.0) as i64), "price": fmt_price(price_of(h)) })).collect()
            }
            _ => vec![],
        };
        let detail = json!({
            "title": src["name"], "brand": src["brand"], "reference": src["reference"], "description": src["description"], "image": src["image"], "specs": src["specs"],
            "offers": offers.iter().enumerate().map(|(i, o)| json!({
                "n": i + 1, "id": o["id"], "store": o["store"], "price": o["price"].as_f64().map(fmt_price), "shipping": o["shipping"],
                "inStock": o["inStock"], "best": best_id.as_deref() == Some(js::gs(o, "id").as_str()), "note": o["note"]
            })).collect::<Vec<_>>(),
            "best": best.as_ref().map(|b| json!({ "price": fmt_price(price_of(b)), "store": b["store"] })),
            "refId": ref_id,
            "keptId": kept.as_ref().map(|k| k["id"].clone()),
            "updatedAt": kept_at.map(short_date).unwrap_or_default(),
            "history": history,
            "footer": FOOTER
        });
        self.o.up().show_widget("product_view", detail);
        json!({ "ok": true, "say": null, "display": js::slice_chars(&format!("Fiche · {}", js::gs(&src, "name")), 60) })
    }

    // --- Commandes par numéro (dernière card affichée) -------------------------------------------------

    fn numbered(&self, n: i64) -> Numbered {
        let shown = self.shown.borrow().clone();
        match shown {
            None => Numbered::Message(json!({ "say": "Je n’ai pas de liste de produits. Dis « je voudrais acheter… » pour en chercher.", "display": "Aucune liste" })),
            Some(Shown::Library(ids)) => {
                let item = (n >= 1).then(|| ids.get(n as usize - 1)).flatten().and_then(|id| self.find_item(id));
                match item {
                    Some(i) => Numbered::Item(i),
                    None => Numbered::Message(json!({ "say": format!("Il n’y a pas de produit {n} dans ta bibliothèque : elle en compte {}.", ids.len()), "display": format!("Pas de produit {n}") })),
                }
            }
            Some(Shown::Search(search)) => {
                let offers = js::arr(&search["offers"]);
                match (n >= 1).then(|| offers.get(n as usize - 1)).flatten() {
                    Some(o) => Numbered::Offer(o.clone(), search.clone()),
                    None => Numbered::Message(json!({ "say": format!("Il n’y a pas de produit {n} dans la dernière liste : elle en compte {}.", offers.len()), "display": format!("Pas de produit {n}") })),
                }
            }
        }
    }

    fn open(&self, n: i64) -> super::Fut<J> {
        let offer = match self.numbered(n) {
            Numbered::Message(m) => return Box::pin(async move { m }),
            Numbered::Offer(o, _) => Some(o),
            Numbered::Item(i) => best_of(&i["offers"]).or_else(|| js::arr(&i["offers"]).into_iter().next()),
        };
        let o = self.o.clone();
        Box::pin(async move { Self::open_offer(&o, offer, Some(n)).await })
    }

    fn detail(&self, n: i64) -> J {
        match self.numbered(n) {
            Numbered::Message(m) => m,
            Numbered::Offer(o, s) => self.show_detail(Some(&s), Some(&o), None),
            Numbered::Item(i) => self.show_detail(None, None, Some(&i)),
        }
    }

    fn keep_number(&self, n: i64) -> J {
        match self.numbered(n) {
            Numbered::Message(m) => m,
            Numbered::Item(i) => json!({ "ok": true, "say": format!("{} est déjà dans ta bibliothèque.", js::gs(&i, "name")), "display": "Déjà gardé" }),
            Numbered::Offer(o, s) => self.keep(&s, &o),
        }
    }

    fn remove_number(&self, n: i64, from_library: bool) -> J {
        let lib_shown = matches!(&*self.shown.borrow(), Some(Shown::Library(_)));
        if !from_library && !lib_shown {
            return json!({ "say": "Pour retirer un produit gardé, dis « supprime le produit 2 de ma bibliothèque ».", "display": "Précise la bibliothèque" });
        }
        match self.numbered(n) {
            Numbered::Item(i) => self.remove(&i),
            Numbered::Offer(o, s) => match self.find_item(&js::gs(&Self::item_from(&s, &o), "id")) {
                Some(i) => self.remove(&i),
                None => json!({ "say": "Ce produit n’est pas dans ta bibliothèque.", "display": "Pas gardé" }),
            },
            Numbered::Message(m) => m,
        }
    }

    fn refresh_number(&self, n: i64) -> J {
        match self.numbered(n) {
            Numbered::Message(m) => m,
            Numbered::Item(i) => self.search(&js::gs(&i, "name"), Some(js::gs(&i, "id"))),
            Numbered::Offer(o, s) => match self.find_item(&js::gs(&Self::item_from(&s, &o), "id")) {
                Some(i) => self.search(&js::gs(&i, "name"), Some(js::gs(&i, "id"))),
                None => {
                    let name = if s["product"].is_object() { js::gs(&s["product"], "name") } else { js::gs(&o, "name") };
                    self.search(&name, None)
                }
            },
        }
    }

    // --- Actions (boutons des cards, par identifiant) ---------------------------------------------------

    pub async fn act(&self, id: &str, action: &str) -> J {
        if let Some(item) = self.find_item(id) {
            return match action {
                "ouvrir" => Self::open_offer(&self.o, best_of(&item["offers"]).or_else(|| js::arr(&item["offers"]).into_iter().next()), None).await,
                "fiche" => self.show_detail(None, None, Some(&item)),
                "actualiser" => self.search(&js::gs(&item, "name"), Some(js::gs(&item, "id"))),
                "supprimer" => self.remove(&item),
                "garder" => json!({ "ok": true, "say": format!("{} est déjà dans ta bibliothèque.", js::gs(&item, "name")) }),
                _ => json!({ "ok": false }),
            };
        }
        let k = self.known.borrow().0.get(id).cloned().or_else(|| self.library_offer(id));
        let Some((offer, search)) = k else { return json!({ "ok": false, "say": "Ce produit n’est plus en mémoire : relance la recherche." }) };
        match action {
            "ouvrir" => Self::open_offer(&self.o, Some(offer), None).await,
            "fiche" => self.show_detail(Some(&search), Some(&offer), None),
            "garder" => self.keep(&search, &offer),
            _ => json!({ "ok": false }),
        }
    }

    /// Offre d'un produit de la bibliothèque (après un redémarrage, `known` est vide).
    fn library_offer(&self, id: &str) -> Option<(J, J)> {
        for item in self.items.borrow().iter() {
            if let Some(offer) = js::arr(&item["offers"]).into_iter().find(|o| js::gs(o, "id") == id) {
                let query = if js::truthy(&item["query"]) { item["query"].clone() } else { item["name"].clone() };
                let product = json!({ "name": item["name"], "brand": item["brand"], "reference": item["reference"], "description": item["description"], "image": item["image"], "specs": if item["specs"].is_array() { item["specs"].clone() } else { json!([]) } });
                return Some((offer, json!({ "query": query, "product": product, "offers": item["offers"] })));
            }
        }
        None
    }

    fn same_search(a: &J, b: &J) -> bool {
        a["query"] == b["query"] && a["at"] == b["at"]
    }

    fn keep(&self, search: &J, offer: &J) -> J {
        let (item, created, _, _) = self.upsert(&Self::item_from(search, offer), js::now_ms());
        let name = js::gs(&item, "name");
        self.o.up().record("produit_garde", json!({ "produit": js::slice_chars(&name, 80), "nouveau": created }));
        let shown = self.shown.borrow().clone();
        match shown {
            Some(Shown::Search(s)) if Self::same_search(&s, search) => self.show_search(&s),
            Some(Shown::Library(_)) => {
                self.library_card();
            }
            _ => {}
        }
        json!({
            "ok": true,
            "say": if created { format!("C'est gardé dans ta bibliothèque de produits : {name}.") } else { format!("{name} était déjà dans ta bibliothèque : prix mis à jour.") },
            "display": if created { "Produit gardé" } else { "Prix mis à jour" }
        })
    }

    fn remove(&self, item: &J) -> J {
        let id = js::gs(item, "id");
        self.items.borrow_mut().retain(|i| js::gs(i, "id") != id);
        self.save();
        let name = js::gs(item, "name");
        self.o.up().record("produit_retire", json!({ "produit": js::slice_chars(&name, 80) }));
        let shown = self.shown.borrow().clone();
        match shown {
            Some(Shown::Library(_)) => {
                self.library_card();
            }
            Some(Shown::Search(s)) => self.show_search(&s),
            None => {}
        }
        json!({ "ok": true, "say": format!("{name} est retiré de ta bibliothèque."), "display": "Produit retiré" })
    }

    async fn open_offer(o: &OrchRef, offer: Option<J>, n: Option<i64>) -> J {
        let Some(offer) = offer.filter(|x| x.is_object()) else { return json!({ "ok": false, "say": "Je n’ai aucune offre à ouvrir pour ce produit.", "display": "Aucune offre" }) };
        let orch = o.up();
        let r = orch
            .local_tool("mcp__system__ouvrir_url", json!({ "url": offer["url"] }), Box::new(|p: J| Box::pin(async move { crate::orch::system::call("ouvrir_url", &p).await.map(|r| r.unwrap_or_else(|e| json!({ "ok": false, "error": e }))).unwrap_or(Value::Null) })))
            .await;
        if r["ok"] == json!(false) || js::gb(&r, "denied") {
            let say = if js::gb(&r, "denied") { "Ouverture refusée.".to_string() } else if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "Je n’ai pas pu ouvrir la page.".into() };
            return json!({ "ok": false, "say": say, "display": "Échec" });
        }
        orch.record("produit_ouvert", json!({ "boutique": offer["store"], "produit": js::slice_chars(&js::gs(&offer, "name"), 80) }));
        let store = js::gs(&offer, "store");
        json!({ "ok": true, "say": null, "display": match n { Some(n) => format!("Produit {n} · {store}"), None => store } })
    }
}
