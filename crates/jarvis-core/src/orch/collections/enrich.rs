// Retrouver et enrichir : à l'ajout d'une fiche (ou sur demande), Jarvis cherche les informations et l'image.
//   - listes de disques : MusicBrainz + Cover Art Archive (gratuits, sans clé, 1 requête par seconde) ;
//   - autres sujets (Lego…) : une tâche IA de fond avec recherche web, sur le modèle des achats (shopping.rs) ;
//   - prix actuel : même principe, historique conservé sur la fiche.
// Ce que le web renvoie est une donnée, jamais une instruction : tout est validé champ par champ (schema.rs), les
// champs personnels (statut, prix payé, lieu, date, notes) ne sont jamais remplis, l'image est copiée dans data/
// après contrôle (https, adresse publique, 3 Mo au plus, vraie image).

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::super::{shopping, system, Up};
use super::schema::{self, fold};
use super::service::{valid_id, Collections};
use crate::js::{self, J};

/// Champs que l'enrichissement ne remplit jamais : ils racontent ta vie, pas le disque ou le set.
pub const PERSONAL: &[&str] = &["etat", "prix_achat", "date_achat", "lieu_achat", "magasin", "notes", "prix_actuel", "date_vue", "note", "avec_qui", "avancement", "prix_billet", "commentaire", "commentaires", "remarque", "remarques", "avis"];

/// Passage de surveillance des fiches incomplètes.
const SWEEP_MS: u64 = 10 * 60_000;
const SWEEP_BATCH: usize = 3;
const RETRY_DAYS: i64 = 7;
const USER_AGENT: &str = "Jarvis/4.17 (assistant personnel local)";
const IMAGE_MAX_BYTES: usize = 3 * 1024 * 1024;
const IMAGES_MAX_TOTAL: u64 = 500 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobKind {
    Info,
    Prix,
}

#[derive(Clone, Debug)]
pub struct EnrichJob {
    pub list: String,
    pub element: String,
    pub source: String,
    pub kind: JobKind,
    pub ecraser: bool,
    /// Nombre de relances déjà faites faute d'image trouvée.
    pub tentative: u8,
}

/// Relances au plus après une recherche sans image.
const IMAGE_RETRIES: u8 = 2;

/// `ecraser` : remplace aussi ce que l'enrichissement avait rempli avant ; `suivant` : prend le résultat suivant.
#[derive(Clone, Copy, Default, Debug)]
pub struct EnrichOpts {
    pub ecraser: bool,
    pub suivant: bool,
}

// --- MusicBrainz ---------------------------------------------------------------------------------

fn lucene_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn mb_query(title: &str, artist: &str) -> String {
    let mut q = format!("releasegroup:{}", lucene_quote(title));
    if !artist.trim().is_empty() {
        q.push_str(&format!(" AND artist:{}", lucene_quote(artist)));
    }
    q
}

/// Les groupes de sorties (albums) d'une réponse MusicBrainz, du mieux noté au moins bien noté.
pub fn parse_musicbrainz(v: &J) -> Vec<J> {
    let mut out: Vec<J> = vec![];
    for g in js::arr(&v["release-groups"]) {
        let id = js::gs(&g, "id");
        if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            continue;
        }
        let artist: String = js::arr(&g["artist-credit"]).iter().map(|c| format!("{}{}", js::gs(c, "name"), js::gs(c, "joinphrase"))).collect();
        let year = js::gs(&g, "first-release-date").get(..4).and_then(|y| y.parse::<i64>().ok());
        let mut tags = js::arr(&g["tags"]);
        tags.sort_by_key(|t| std::cmp::Reverse(t["count"].as_i64().unwrap_or(0)));
        let genre = tags.first().map(|t| js::gs(t, "name")).unwrap_or_default();
        out.push(json!({
            "id": id, "titre": schema::clean_text(&js::gs(&g, "title"), 120), "artiste": schema::clean_text(&artist, 120),
            "annee": year, "genre": schema::clean_text(&genre, 40), "score": g["score"].as_i64().unwrap_or(0)
        }));
    }
    out.sort_by_key(|c| std::cmp::Reverse(c["score"].as_i64().unwrap_or(0)));
    out.truncate(5);
    out
}

/// Disques d'une recherche MusicBrainz par code-barres (sorties) : groupe de sorties, titre, artiste, année.
pub fn parse_mb_barcode(v: &J) -> Vec<J> {
    let mut out: Vec<J> = vec![];
    for r in js::arr(&v["releases"]) {
        let id = js::gs(&r["release-group"], "id");
        if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') || out.iter().any(|c| js::gs(c, "id") == id) {
            continue;
        }
        let artist: String = js::arr(&r["artist-credit"]).iter().map(|c| format!("{}{}", js::gs(c, "name"), js::gs(c, "joinphrase"))).collect();
        let year = js::gs(&r, "date").get(..4).and_then(|y| y.parse::<i64>().ok());
        out.push(json!({
            "id": id, "titre": schema::clean_text(&js::gs(&r, "title"), 120), "artiste": schema::clean_text(&artist, 120),
            "annee": year, "genre": "", "score": 100, "codeBarre": true
        }));
    }
    out.truncate(5);
    out
}

/// Adresse d'une page marchande lue par l'IA : https, adresse publique (jamais une adresse locale).
pub fn shop_url(v: &J) -> Option<String> {
    shopping::safe_url(v).filter(|u| u.starts_with("https://") && host_is_public(u))
}

pub fn title_close(a: &str, b: &str) -> bool {
    let (fa, fb) = (fold(a), fold(b));
    !fa.is_empty() && !fb.is_empty() && (fa == fb || fa.contains(&fb) || fb.contains(&fa))
}

// --- Images ---------------------------------------------------------------------------------------

/// Extension d'une vraie image d'après ses premiers octets (jamais d'après ce que dit le serveur).
pub fn sniff_image(b: &[u8]) -> Option<&'static str> {
    if b.len() > 12 && b[..3] == [0xFF, 0xD8, 0xFF] {
        Some("jpg")
    } else if b.len() > 12 && b[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        Some("png")
    } else if b.len() > 12 && (b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        Some("gif")
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// https vers un nom de domaine public : pas d'adresse IP, pas de localhost ni de réseau local.
pub fn host_is_public(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else { return false };
    if u.scheme() != "https" {
        return false;
    }
    let Some(host) = u.host_str() else { return false };
    let host = host.to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') || !host.contains('.') {
        return false;
    }
    !["localhost", ".local", ".localhost", ".internal", ".lan", ".home", ".corp", ".intranet"].iter().any(|s| host == s.trim_start_matches('.') || host.ends_with(s))
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| match e.metadata() { Ok(m) if m.is_dir() => dir_size(&e.path()), Ok(m) => m.len(), Err(_) => 0 }).sum())
        .unwrap_or(0)
}

pub(super) async fn http_get(url: &str) -> Result<reqwest::Response, String> {
    crate::net::http().get(url).header("User-Agent", USER_AGENT).timeout(Duration::from_secs(20)).send().await.map_err(|e| format!("réseau : {e}"))
}

/// MusicBrainz limite le débit (503 / 429) : jusqu'à trois essais espacés.
async fn mb_get_json(url: &str) -> Result<J, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            js::sleep_ms(2500 * attempt).await;
        }
        let resp = http_get(url).await?;
        let code = resp.status().as_u16();
        if resp.status().is_success() {
            return resp.json().await.map_err(|e| format!("réponse illisible : {e}"));
        }
        last = format!("MusicBrainz a répondu {code}");
        if code != 503 && code != 429 {
            break;
        }
    }
    Err(last)
}

/// Télécharge une image : `trusted` = adresse que Jarvis a construite lui-même (Cover Art Archive).
pub async fn download_image(url: &str, trusted: bool) -> Result<(Vec<u8>, &'static str), String> {
    if !url.starts_with("https://") || (!trusted && !host_is_public(url)) {
        return Err("adresse d'image refusée".into());
    }
    let mut resp = http_get(url).await?;
    if !resp.status().is_success() {
        return Err(format!("image introuvable ({})", resp.status().as_u16()));
    }
    if !host_is_public(resp.url().as_str()) {
        return Err("redirection refusée".into());
    }
    if resp.content_length().map(|l| l as usize > IMAGE_MAX_BYTES).unwrap_or(false) {
        return Err("image trop lourde".into());
    }
    let mut bytes: Vec<u8> = vec![];
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("réseau : {e}"))? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > IMAGE_MAX_BYTES {
            return Err("image trop lourde".into());
        }
    }
    let ext = sniff_image(&bytes).ok_or("ce n'est pas une image")?;
    Ok((bytes, ext))
}

// --- Gabarits et consignes -------------------------------------------------------------------------

/// « album {titre} {artiste} » → texte ; `encode` : valeurs encodées pour une adresse web.
pub fn expand_template(tpl: &str, list: &J, item: &J, encode: bool) -> String {
    let out = crate::re!(r"\{([a-z0-9_]+)\}").replace_all(tpl, |c: &regex::Captures| {
        let v = schema::display(list, item, &c[1]);
        if encode { urlencoding::encode(&v).into_owned() } else { v }
    });
    if encode { out.to_string() } else { crate::re!(r"\s+").replace_all(&out, " ").trim().to_string() }
}

fn personal_keys(list: &J) -> Vec<String> {
    PERSONAL.iter().map(|s| s.to_string()).chain(list["etat"].as_str().map(String::from)).collect()
}

fn identity(list: &J, item: &J) -> String {
    let code = js::gs(item, "codeBarre");
    if js::gb(item, "titreProvisoire") && !code.is_empty() {
        return format!("le produit de code-barres EAN/UPC {code} (cherche d'abord ce code : il identifie exactement le produit, donne-lui son vrai nom)");
    }
    let mut parts = vec![format!("« {} »", schema::title_of(list, item))];
    if !code.is_empty() {
        parts.push(format!("code-barres EAN/UPC {code}"));
    }
    let sub = schema::subtitle_of(list, item);
    if !sub.is_empty() {
        parts.push(sub);
    }
    for f in js::arr(&list["champs"]).iter().filter(|f| js::gs(f, "type") == "lien") {
        let url = schema::display(list, item, &js::gs(f, "cle"));
        if !url.is_empty() {
            parts.push(format!("adresse de la page du produit : {url} (consulte-la d'abord ; si elle ne s'ouvre pas, déduis le produit de son adresse)"));
        }
    }
    parts.join(", ")
}

pub fn info_prompt(list: &J, item: &J, tag: &str) -> String {
    let personal = personal_keys(list);
    let fields: Vec<String> = js::arr(&list["champs"])
        .iter()
        .filter(|f| !personal.contains(&js::gs(f, "cle")) && js::gs(f, "type") != "image")
        .map(|f| {
            let choix = if js::gs(f, "type") == "choix" { format!(" (choix : {})", js::str_list(&f["choix"]).join(" / ")) } else { String::new() };
            format!("{} ({}{}) : {}", js::gs(f, "cle"), js::gs(f, "type"), choix, js::gs(f, "libelle"))
        })
        .collect();
    let consigne = js::gs(&list["enrichissement"], "consigne");
    [
        format!("Je tiens une liste « {} » de {}. Retrouve sur le web ce {} : {}.", js::gs(list, "nom"), js::gs(&list["noms"], "pluriel"), js::gs(&list["noms"], "singulier"), identity(list, item)),
        if consigne.is_empty() { String::new() } else { format!("Ce qui m'intéresse : {consigne}.") },
        "Utilise la recherche web (WebSearch puis WebFetch) et recoupe au moins deux sources fiables.".into(),
        "Remplis chaque champ séparément et à sa place (prix, marque, caractéristiques…) : jamais plusieurs informations dans un seul champ texte. Donne toujours une image si tu en trouves une.".into(),
        "Ne devine ni n'invente jamais une valeur : si tu ne l'as pas vue sur une page que tu as consultée, laisse le champ absent. Ne remplis jamais un statut, un prix payé, une date ou un lieu d'achat, ni des notes.".into(),
        "Réponds uniquement par un objet JSON valide, sans texte autour : {\"valeurs\": {\"clé du champ\": valeur, …}, \"image\": \"adresse https d'une photo ou pochette lue sur une page consultée, sinon vide\", \"source\": \"adresse https de la page principale\", \"confiance\": nombre de 0 à 1}.".into(),
        format!("Champs possibles, par clé : {}.", fields.join(" ; ")),
        format!("[liste-info-{tag}]"),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

pub fn price_prompt(query: &str, tag: &str) -> String {
    [
        format!("Cherche le prix actuel en France, neuf, de : {query}."),
        "Utilise la recherche web (WebSearch puis WebFetch) : retiens la meilleure offre fiable d'un marchand que tu as réellement consulté.".into(),
        "Ne devine ni n'invente jamais un prix ni une adresse : prix vaut null si tu ne l'as pas vu.".into(),
        "Réponds uniquement par un objet JSON valide, sans texte autour : {\"prix\": 129.9, \"boutique\": \"nom du marchand\", \"url\": \"https://…\"}.".into(),
        "url : l'adresse https de la page de ce produit chez ce marchand, celle où tu as lu le prix (jamais une page d'accueil ni une page de résultats de recherche).".into(),
        format!("[liste-prix-{tag}]"),
    ]
    .join(" ")
}

/// Ce que la réponse web apporte, validé : valeurs de champs, image, page source, confiance.
pub struct WebFound {
    pub values: Map<String, Value>,
    pub image: Option<String>,
    pub source: Option<String>,
    pub confiance: f64,
}

pub fn parse_web_result(list: &J, data: &J) -> WebFound {
    let personal = personal_keys(list);
    let raw = if data["valeurs"].is_object() { &data["valeurs"] } else { data };
    let mut values = Map::new();
    if let Some(obj) = raw.as_object() {
        for (k, v) in obj {
            if personal.contains(k) {
                continue;
            }
            let Some(f) = schema::field(list, k) else { continue };
            if js::gs(f, "type") == "image" {
                continue;
            }
            if let Ok(nv) = schema::normalize_value(f, v) {
                if !nv.is_null() {
                    values.insert(k.clone(), nv);
                }
            }
        }
    }
    let image = Some(shopping::safe_image(&data["image"])).filter(|u| !u.is_empty() && host_is_public(u));
    let source = shopping::safe_url(&data["source"]);
    let confiance = data["confiance"].as_f64().map(|c| c.clamp(0.0, 1.0)).unwrap_or(0.7);
    WebFound { values, image, source, confiance }
}

/// Fusionne des valeurs trouvées : ce que tu as saisi n'est jamais écrasé ; ce que l'enrichissement a rempli
/// l'est si `ecraser`. Rend les clés modifiées.
pub fn merge_values(vals: &mut Map<String, Value>, filled: &mut Vec<String>, patch: &Map<String, Value>, ecraser: bool, overwritable: &[String]) -> Vec<String> {
    let mut changed = vec![];
    for (k, v) in patch {
        let may = !vals.contains_key(k) || overwritable.contains(k) || (ecraser && filled.contains(k));
        if may && vals.get(k) != Some(v) {
            vals.insert(k.clone(), v.clone());
            changed.push(k.clone());
        }
        if may && !filled.contains(k) {
            filled.push(k.clone());
        }
    }
    changed
}

impl Collections {
    fn announce(&self, source: &str, text: String) {
        if source == "voix" {
            if let Some(o) = self.o.upgrade() {
                o.emit(json!({ "type": "say", "text": text }));
            }
        }
    }

    fn element_of(&self, list_id: &str, element_id: &str) -> Option<(J, J)> {
        let list = self.get(list_id)?;
        let item = js::arr(&list["elements"]).into_iter().find(|e| js::gs(e, "id") == element_id)?;
        Some((list, item))
    }

    /// La fiche cesse d'être un brouillon (rien trouvé, ou échec).
    fn clear_draft(&self, list_id: &str, element_id: &str) {
        let Some(mut list) = self.get(list_id) else { return };
        let mut changed = false;
        if let Some(items) = list["elements"].as_array_mut() {
            if let Some(it) = items.iter_mut().find(|e| js::gs(e, "id") == element_id) {
                changed = js::gb(it, "brouillon");
                it["brouillon"] = json!(false);
            }
        }
        if changed && self.save_list(&mut list).is_ok() {
            self.changed(list_id, Some(element_id), "enrichissement_vide");
        }
    }

    /// Lance l'enrichissement en tâche de fond. false si impossible (orchestrateur absent).
    pub fn enrich_soon(&self, list_id: &str, element_id: &str, source: &str, opts: EnrichOpts) -> bool {
        let Some(o) = self.o.upgrade() else { return false };
        let (l, e, s) = (list_id.to_string(), element_id.to_string(), source.to_string());
        js::spawn(async move {
            let _ = o.collections.enrich(&l, &e, &s, opts).await;
        });
        true
    }

    pub fn has_provider(list: &J) -> bool {
        matches!(js::gs(&list["enrichissement"], "fournisseur").as_str(), "disque" | "serie" | "web")
    }

    /// Liste sans fournisseur mais demandée sur mesure (Nomade) : l'IA complète quand même les fiches qui ont des trous.
    pub fn can_fill(list: &J) -> bool {
        Self::has_provider(list) || list["origine"].is_object()
    }

    /// La fiche a au moins un champ à retrouver (hors saisie personnelle, image et champs réservés à la vie de l'utilisateur) vide.
    pub fn has_gaps(list: &J, vals: &Map<String, Value>) -> bool {
        let personal = personal_keys(list);
        js::arr(&list["champs"]).iter().any(|f| {
            let k = js::gs(f, "cle");
            js::gs(f, "type") != "image" && !personal.contains(&k) && vals.get(&k).is_none_or(|v| v.is_null())
        })
    }

    /// Surveillance : une minute après le démarrage puis toutes les 10 minutes, les fiches incomplètes sont complétées par l'IA.
    pub fn start(&self) {
        let w = self.o.clone();
        self.sweep.every(SWEEP_MS, move || {
            if let Some(o) = w.upgrade() {
                o.collections.complete_gaps();
            }
        });
        let w = self.o.clone();
        js::set_timeout(60_000, move || {
            if let Some(o) = w.upgrade() {
                o.collections.complete_gaps();
            }
        });
    }

    /// Cherche les fiches incomplètes (au plus `SWEEP_BATCH` par passage) et lance la recherche IA. Une fiche déjà essayée
    /// depuis moins de `RETRY_DAYS` jours n'est pas relancée : un champ introuvable ne boucle pas.
    pub fn complete_gaps(&self) -> usize {
        if self.o.upgrade().is_none() {
            return 0;
        }
        let now = js::now_ms();
        let mut started = 0;
        for mut list in self.all() {
            if started >= SWEEP_BATCH || !Self::can_fill(&list) {
                continue;
            }
            let lid = js::gs(&list, "id");
            // Liste demandée sur mesure : même statut que sur le site (Souhaité / Possédé), une image par fiche, recherche IA.
            if list["origine"].is_object() {
                let mut aligned = list.clone();
                if schema::align_with_site(&mut aligned) {
                    let etat = js::gs(&aligned, "etat");
                    let image = js::gs(&aligned, "image");
                    let mut patch = json!({ "etat": etat, "image": image, "enrichissement": aligned["enrichissement"], "choix": { etat.clone(): ["Souhaité", "Possédé"] } });
                    let mut add = vec![];
                    for k in [&etat, &image] {
                        if schema::field(&list, k).is_none() && !add.iter().any(|f: &J| js::gs(f, "cle") == *k) {
                            if let Some(f) = schema::field(&aligned, k) {
                                add.push(f.clone());
                            }
                        }
                    }
                    patch["ajouter"] = json!(add);
                    if let Err(e) = self.update_schema(&lid, &patch, "auto") {
                        (self.log)(format!("Collections : modèle de « {} » non aligné : {e}", js::gs(&list, "nom")));
                    }
                    if let Some(l) = self.get(&lid) {
                        list = l;
                    }
                }
                // Informations que l'IA avait rangées dans un champ personnel (commentaire) : retirées, la fiche sera recherchée de nouveau.
                let personal = personal_keys(&list);
                let mut cleaned = false;
                if let Some(items) = list["elements"].as_array_mut() {
                    for it in items.iter_mut() {
                        let bad: Vec<String> = js::str_list(&it["rempli"]).into_iter().filter(|k| personal.contains(k)).collect();
                        for k in bad {
                            if let Some(v) = it["valeurs"].as_object_mut() {
                                v.remove(&k);
                            }
                            it["rempli"] = json!(js::str_list(&it["rempli"]).into_iter().filter(|x| *x != k).collect::<Vec<_>>());
                            it["completeLe"] = json!(0);
                            cleaned = true;
                        }
                    }
                }
                if cleaned {
                    let _ = self.save_list(&mut list);
                }
            }
            let view = list.clone();
            let mut picked: Vec<String> = vec![];
            if let Some(items) = list["elements"].as_array_mut() {
                for it in items.iter_mut() {
                    if started + picked.len() >= SWEEP_BATCH {
                        break;
                    }
                    let vals = it["valeurs"].as_object().cloned().unwrap_or_default();
                    let tried = it["completeLe"].as_i64().unwrap_or(0);
                    let busy = js::gb(it, "brouillon") || self.enrich_jobs.borrow().iter().any(|(_, j)| j.list == lid && j.element == js::gs(it, "id"));
                    let no_image = view["image"].is_string() && js::gs(it, "image").is_empty();
                    if busy || !(Self::has_gaps(&view, &vals) || no_image) || now - tried < RETRY_DAYS * 86_400_000 {
                        continue;
                    }
                    it["completeLe"] = json!(now);
                    picked.push(js::gs(it, "id"));
                }
            }
            if picked.is_empty() || self.save_list(&mut list).is_err() {
                continue;
            }
            for id in &picked {
                self.enrich_soon(&lid, id, "auto", EnrichOpts::default());
            }
            (self.log)(format!("Collections : {} fiche{} incomplète{} de « {} », recherche IA lancée", picked.len(), if picked.len() > 1 { "s" } else { "" }, if picked.len() > 1 { "s" } else { "" }, js::gs(&list, "nom")));
            started += picked.len();
        }
        started
    }

    /// Cherche les informations et l'image d'une fiche.
    pub async fn enrich(&self, list_id: &str, element_id: &str, source: &str, opts: EnrichOpts) -> Result<J, String> {
        let (list, item) = self.element_of(list_id, element_id).ok_or("Fiche introuvable.")?;
        let title = schema::title_of(&list, &item);
        match js::gs(&list["enrichissement"], "fournisseur").as_str() {
            "disque" | "serie" => match self.enrich_found(&list, &item, opts).await {
                Ok(Some(found)) => {
                    let sub = [js::gs(&found, "artiste"), js::gs(&found, "annee")].into_iter().filter(|s| !s.is_empty() && s != "null").collect::<Vec<_>>().join(", ");
                    self.announce(source, format!("J'ai retrouvé {title}{}{}.", if sub.is_empty() { String::new() } else { format!(" : {sub}") }, if js::gb(&found, "image") { ", avec son image" } else { "" }));
                    Ok(json!({ "ok": true, "trouve": true, "say": format!("Fiche complétée : {title}.") }))
                }
                Ok(None) if !opts.suivant => {
                    // MusicBrainz ne connaît pas ce disque : l'IA essaie avec le web.
                    self.start_job(&list, &item, source, JobKind::Info, opts.ecraser);
                    Ok(json!({ "ok": true, "trouve": false, "say": format!("Je ne trouve pas « {title} » dans le catalogue gratuit : je demande à l'IA de chercher sur le web.") }))
                }
                Ok(None) => Ok(json!({ "ok": false, "erreur": "Pas d'autre résultat." })),
                Err(e) => {
                    (self.log)(format!("Collections enrichissement : {e}"));
                    self.clear_draft(list_id, element_id);
                    self.announce(source, format!("Je n'ai pas pu chercher les informations sur {title} : {e}."));
                    Err(e)
                }
            },
            "web" => {
                self.start_job(&list, &item, source, JobKind::Info, opts.ecraser);
                Ok(json!({ "ok": true, "trouve": false, "say": format!("Je cherche les informations sur {title}.") }))
            }
            _ if Self::can_fill(&list) => {
                let vals = item["valeurs"].as_object().cloned().unwrap_or_default();
                if !Self::has_gaps(&list, &vals) && !opts.ecraser {
                    self.clear_draft(list_id, element_id);
                    return Ok(json!({ "ok": true, "trouve": false, "say": format!("La fiche de {title} est déjà complète.") }));
                }
                self.start_job(&list, &item, source, JobKind::Info, opts.ecraser);
                Ok(json!({ "ok": true, "trouve": false, "say": format!("Je cherche les informations manquantes sur {title}.") }))
            }
            _ => {
                self.clear_draft(list_id, element_id);
                Err("Cette liste n'a pas d'enrichissement automatique.".into())
            }
        }
    }

    /// Le fournisseur de catalogue de la liste : MusicBrainz (disques) ou TVmaze (séries).
    async fn enrich_found(&self, list: &J, item: &J, opts: EnrichOpts) -> Result<Option<J>, String> {
        match js::gs(&list["enrichissement"], "fournisseur").as_str() {
            "serie" => self.enrich_series(list, item, opts).await,
            _ => self.enrich_disc(list, item, opts).await,
        }
    }

    /// MusicBrainz + Cover Art Archive. Some(résumé) si une fiche a été complétée.
    async fn enrich_disc(&self, list: &J, item: &J, opts: EnrichOpts) -> Result<Option<J>, String> {
        let title = schema::display(list, item, &js::gs(list, "titre"));
        let artist = schema::display(list, item, "artiste");
        let mut cands = js::arr(&item["candidats"]);
        let mut idx = item["candidatIndex"].as_u64().unwrap_or(0) as usize;
        if opts.suivant && !cands.is_empty() {
            idx = (idx + 1) % cands.len();
        } else if cands.is_empty() || opts.ecraser {
            // Code-barres scanné : la sortie exacte d'abord, puis le titre si le code est inconnu.
            let code = js::gs(item, "codeBarre");
            cands = vec![];
            if !code.is_empty() {
                let url = format!("https://musicbrainz.org/ws/2/release/?query=barcode:{code}&fmt=json&limit=5");
                cands = parse_mb_barcode(&mb_get_json(&url).await?);
                js::sleep_ms(1100).await; // MusicBrainz : une requête par seconde
            }
            if cands.is_empty() && !(js::gb(item, "titreProvisoire") && !code.is_empty()) {
                let url = format!("https://musicbrainz.org/ws/2/release-group/?query={}&fmt=json&limit=5", urlencoding::encode(&mb_query(&title, &artist)));
                let data = mb_get_json(&url).await?;
                cands = parse_musicbrainz(&data);
                js::sleep_ms(1100).await;
            }
            idx = 0;
        }
        let Some(c) = cands.get(idx).cloned() else { return Ok(None) };
        if c["score"].as_i64().unwrap_or(0) < 60 {
            return Ok(None);
        }
        let mut patch = Map::new();
        for (key, from) in [("artiste", "artiste"), ("annee", "annee"), ("genre", "genre")] {
            if let Some(f) = schema::field(list, key) {
                let v = &c[from];
                if !v.is_null() && !js::s(v).is_empty() {
                    if let Ok(nv) = schema::normalize_value(f, v) {
                        if !nv.is_null() {
                            patch.insert(key.to_string(), nv);
                        }
                    }
                }
            }
        }
        // Fiche créée par un code-barres : le vrai titre remplace le nom provisoire.
        if js::gb(item, "titreProvisoire") {
            let tk = js::gs(list, "titre");
            if let (Some(f), false) = (schema::field(list, &tk), js::gs(&c, "titre").is_empty()) {
                if let Ok(nv) = schema::normalize_value(f, &c["titre"]) {
                    patch.insert(tk, nv);
                }
            }
        }
        let cover_url = format!("https://coverartarchive.org/release-group/{}/front-500", js::gs(&c, "id"));
        let image = match download_image(&cover_url, true).await {
            Ok((bytes, ext)) => self.store_image(&js::gs(list, "id"), &js::gs(item, "id"), &bytes, ext).ok(),
            Err(_) => None,
        };
        let close = js::gb(&c, "codeBarre") || title_close(&title, &js::gs(&c, "titre"));
        let conf = c["score"].as_f64().unwrap_or(0.0) / 100.0 * if close { 1.0 } else { 0.5 };
        let meta = json!({ "origine": "MusicBrainz", "confiance": (conf * 100.0).round() / 100.0, "candidats": cands, "candidatIndex": idx });
        self.apply_enrichment(&js::gs(list, "id"), &js::gs(item, "id"), patch, image.clone(), meta, opts.ecraser)?;
        Ok(Some(json!({ "artiste": c["artiste"], "annee": c["annee"], "image": image.is_some() })))
    }

    // --- Copie locale des images -------------------------------------------------------------------

    pub fn store_image(&self, list_id: &str, element_id: &str, bytes: &[u8], ext: &str) -> Result<String, String> {
        if !valid_id(list_id) || !valid_id(element_id) || !["jpg", "png", "gif", "webp"].contains(&ext) {
            return Err("identifiant invalide".into());
        }
        if dir_size(&self.images_dir()) + bytes.len() as u64 > IMAGES_MAX_TOTAL {
            return Err("le cache d'images est plein (500 Mo)".into());
        }
        let dir = self.images_dir().join(list_id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for other in ["jpg", "png", "gif", "webp"] {
            let _ = std::fs::remove_file(dir.join(format!("{element_id}.{other}")));
        }
        std::fs::write(dir.join(format!("{element_id}.{ext}")), bytes).map_err(|e| e.to_string())?;
        Ok(format!("local:{ext}#{}", js::now_ms() / 1000))
    }

    /// Supprime les images dont la fiche n'existe plus (au démarrage : un « annule » récent reste possible).
    pub fn prune_images(&self) {
        let Ok(rd) = std::fs::read_dir(self.images_dir()) else { return };
        for d in rd.filter_map(|e| e.ok()) {
            let list_id = d.file_name().to_string_lossy().to_string();
            let Some(list) = self.get(&list_id) else {
                let _ = std::fs::remove_dir_all(d.path());
                continue;
            };
            let ids: Vec<String> = js::arr(&list["elements"]).iter().map(|e| js::gs(e, "id")).collect();
            if let Ok(files) = std::fs::read_dir(d.path()) {
                for f in files.filter_map(|e| e.ok()) {
                    let stem = f.path().file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    if !ids.contains(&stem) {
                        let _ = std::fs::remove_file(f.path());
                    }
                }
            }
        }
    }

    /// Écrit le résultat d'un enrichissement dans la fiche.
    pub(super) fn apply_enrichment(&self, list_id: &str, element_id: &str, patch: Map<String, Value>, image: Option<String>, meta: J, ecraser: bool) -> Result<(), String> {
        let mut list = self.get(list_id).ok_or("Liste introuvable.")?;
        let title_key = js::gs(&list, "titre");
        let updated: J;
        {
            let items = list["elements"].as_array_mut().ok_or("Liste invalide.")?;
            let item = items.iter_mut().find(|e| js::gs(e, "id") == element_id).ok_or("Fiche introuvable.")?;
            let mut vals = item["valeurs"].as_object().cloned().unwrap_or_default();
            let mut filled = js::str_list(&item["rempli"]);
            let overwritable: Vec<String> = if js::gb(item, "titreProvisoire") { vec![title_key.clone()] } else { vec![] };
            let changed = merge_values(&mut vals, &mut filled, &patch, ecraser, &overwritable);
            if changed.contains(&title_key) {
                item["titreProvisoire"] = json!(false);
            }
            item["valeurs"] = Value::Object(vals);
            item["rempli"] = json!(filled);
            if let Some(img) = image {
                item["image"] = json!(img);
            }
            for k in ["origine", "confiance", "candidats", "candidatIndex"] {
                if !meta[k].is_null() {
                    item[k] = meta[k].clone();
                }
            }
            item["brouillon"] = json!(false);
            item["majLe"] = json!(js::iso_now());
            updated = item.clone();
        }
        self.save_list(&mut list)?;
        self.record("element_enrichi", json!({ "liste": list_id, "element": element_id, "origine": updated["origine"] }));
        self.changed(list_id, Some(element_id), "element_enrichi");
        Ok(())
    }

    // --- Tâches IA de fond (web, prix) -------------------------------------------------------------

    fn start_job(&self, list: &J, item: &J, source: &str, kind: JobKind, ecraser: bool) {
        self.start_job_n(list, item, source, kind, ecraser, 0);
    }

    fn start_job_n(&self, list: &J, item: &J, source: &str, kind: JobKind, ecraser: bool, tentative: u8) {
        let Some(o) = self.o.upgrade() else { return };
        let tag = format!("{}-{}", js::now_ms(), js::rand_below(10_000));
        let prompt = match kind {
            JobKind::Info => info_prompt(list, item, &tag),
            JobKind::Prix => {
                let tpl = js::arr(&list["actions"]).iter().find(|a| js::gs(a, "type") == "prix").map(|a| js::gs(a, "gabarit")).unwrap_or_default();
                let q = if tpl.is_empty() { identity(list, item) } else { expand_template(&tpl, list, item, false) };
                price_prompt(&q, &tag)
            }
        };
        self.enrich_jobs.borrow_mut().push((prompt.clone(), EnrichJob { list: js::gs(list, "id"), element: js::gs(item, "id"), source: source.to_string(), kind, ecraser, tentative }));
        o.enqueue_ai(prompt, json!({ "background": true }));
    }

    /// Fin d'une tâche de fond d'enrichissement ou de prix : true si c'était l'une des nôtres.
    pub fn enrich_done(&self, text: &str, answer: &str, ok: bool) -> bool {
        let job = {
            let mut jobs = self.enrich_jobs.borrow_mut();
            let Some(i) = jobs.iter().position(|(p, _)| p == text) else { return false };
            jobs.remove(i).1
        };
        let Some((list, item)) = self.element_of(&job.list, &job.element) else { return true };
        let title = schema::title_of(&list, &item);
        let data = if ok { shopping::read_offers_data(Path::new(""), answer) } else { None };
        // Une liste sans champ image ne relance pas la recherche pour une image qu'elle ne peut pas afficher.
        let wants_image = list["image"].is_string();
        let Some(data) = data else {
            if job.kind == JobKind::Info && wants_image && js::gs(&item, "image").is_empty() && job.tentative < IMAGE_RETRIES {
                (self.log)(format!("Collections : pas de résultat pour {title}, nouvelle recherche ({}/{IMAGE_RETRIES})", job.tentative + 1));
                self.start_job_n(&list, &item, &job.source, JobKind::Info, job.ecraser, job.tentative + 1);
                return true;
            }
            self.clear_draft(&job.list, &job.element);
            self.announce(&job.source, if job.kind == JobKind::Prix { format!("Je n'ai pas trouvé le prix de {title}.") } else { format!("Je n'ai pas trouvé d'informations fiables sur {title} : la fiche reste telle quelle.") });
            return true;
        };
        let Some(o) = self.o.upgrade() else { return true };
        match job.kind {
            JobKind::Info => {
                let found = parse_web_result(&list, &data);
                js::spawn(async move {
                    let c = &o.collections;
                    let image = match &found.image {
                        Some(u) => match download_image(u, false).await {
                            Ok((bytes, ext)) => c.store_image(&job.list, &job.element, &bytes, ext).ok(),
                            Err(_) => None,
                        },
                        None => None,
                    };
                    let retry = wants_image && image.is_none() && js::gs(&item, "image").is_empty() && job.tentative < IMAGE_RETRIES;
                    if found.values.is_empty() && image.is_none() {
                        if retry {
                            (c.log)(format!("Collections : pas d'image pour {title}, nouvelle recherche ({}/{IMAGE_RETRIES})", job.tentative + 1));
                            c.start_job_n(&list, &item, &job.source, JobKind::Info, job.ecraser, job.tentative + 1);
                            return;
                        }
                        c.clear_draft(&job.list, &job.element);
                        c.announce(&job.source, format!("Je n'ai pas trouvé d'informations fiables sur {title} : la fiche reste telle quelle."));
                        return;
                    }
                    let origin = found.source.clone().map(|s| s.replace("https://", "").split('/').next().unwrap_or("").to_string()).unwrap_or_else(|| "le web".into());
                    let meta = json!({ "origine": origin, "confiance": (found.confiance * 100.0).round() / 100.0 });
                    let n = found.values.len();
                    match c.apply_enrichment(&job.list, &job.element, found.values, image.clone(), meta, job.ecraser) {
                        Ok(_) => c.announce(&job.source, format!("J'ai complété la fiche de {title} : {n} information{}{}.", if n > 1 { "s" } else { "" }, if image.is_some() { " et l'image" } else { "" })),
                        Err(e) => (c.log)(format!("Collections enrichissement : {e}")),
                    }
                    // Infos trouvées mais pas d'image : on recommence (au plus 2 fois) pour en trouver une.
                    if retry {
                        if let Some((l2, i2)) = c.element_of(&job.list, &job.element) {
                            c.start_job_n(&l2, &i2, &job.source, JobKind::Info, job.ecraser, job.tentative + 1);
                        }
                    }
                });
            }
            JobKind::Prix => {
                let price = shopping::parse_price(&data["prix"]);
                let shop = schema::clean_text(&js::gs(&data, "boutique"), 60);
                let url = shop_url(&data["url"]);
                let Some(price) = price else {
                    self.announce("voix", format!("Je n'ai pas trouvé de prix fiable pour {title}."));
                    return true;
                };
                match self.record_price(&job.list, &job.element, price, &shop, url.as_deref()) {
                    Ok(before) => {
                        let change = match before {
                            Some(b) if (b - price).abs() < 0.005 => " Le prix n'a pas changé.".to_string(),
                            Some(b) => format!(" Avant : {}{}.", shopping::spoken_price(b), if price < b { ", ça a baissé" } else { ", ça a augmenté" }),
                            None => String::new(),
                        };
                        o.emit(json!({ "type": "say", "text": format!("Le prix actuel de {title} : {}{}.{change}", shopping::spoken_price(price), if shop.is_empty() { String::new() } else { format!(" chez {shop}") }) }));
                    }
                    Err(e) => (self.log)(format!("Collections prix : {e}")),
                }
            }
        }
        true
    }

    /// Note un prix relevé : champ « prix actuel » s'il existe, et historique (20 relevés au plus). Rend le prix précédent.
    pub(super) fn record_price(&self, list_id: &str, element_id: &str, price: f64, shop: &str, url: Option<&str>) -> Result<Option<f64>, String> {
        let mut list = self.get(list_id).ok_or("Liste introuvable.")?;
        let has_field = schema::field(&list, "prix_actuel").is_some();
        let before;
        {
            let items = list["elements"].as_array_mut().ok_or("Liste invalide.")?;
            let item = items.iter_mut().find(|e| js::gs(e, "id") == element_id).ok_or("Fiche introuvable.")?;
            let mut hist = js::arr(&item["historiquePrix"]);
            before = hist.last().and_then(|h| js::num(&h["prix"]));
            let mut entry = json!({ "date": js::local_now().format("%Y-%m-%d").to_string(), "prix": price, "boutique": shop });
            if let Some(u) = url {
                entry["url"] = json!(u); // page du produit chez le marchand
            }
            hist.push(entry);
            if hist.len() > 20 {
                hist.remove(0);
            }
            item["historiquePrix"] = json!(hist);
            if has_field {
                if let Some(vals) = item["valeurs"].as_object_mut() {
                    vals.insert("prix_actuel".into(), json!(price));
                }
            }
            item["majLe"] = json!(js::iso_now());
        }
        self.save_list(&mut list)?;
        self.record("element_prix_releve", json!({ "liste": list_id, "element": element_id }));
        self.changed(list_id, Some(element_id), "prix_releve");
        Ok(before)
    }

    /// Ouvre la page du produit chez le marchand d'un prix relevé (n° dans l'historique). L'adresse vient
    /// de la fiche, jamais de la fenêtre.
    pub async fn open_price_link(&self, list_id: &str, element_id: &str, index: usize) -> Result<J, String> {
        let (_, item) = self.element_of(list_id, element_id).ok_or("Fiche introuvable.")?;
        let entry = js::arr(&item["historiquePrix"]).get(index).cloned().ok_or("Prix introuvable.")?;
        let url = shop_url(&entry["url"]).ok_or("Ce prix n'a pas de lien vers la boutique.")?;
        let o = self.o.up();
        let r = o.local_tool("mcp__system__ouvrir_url", json!({ "url": url }), Box::new(|prm| Box::pin(async move { system::ouvrir_url(&prm).await.unwrap_or_else(|e| json!({ "ok": false, "error": e })) }))).await;
        if r["ok"] == json!(false) {
            return Ok(json!({ "ok": false, "say": if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "Ouverture refusée.".into() } }));
        }
        let shop = js::gs(&entry, "boutique");
        Ok(json!({ "ok": true, "say": if shop.is_empty() { "Page du produit ouverte.".to_string() } else { format!("Page du produit chez {shop}.") } }))
    }

    // --- Boutons spéciaux d'une liste -----------------------------------------------------------------

    /// Exécute une action déclarée par la liste (écouter, ouvrir une page, prix actuel, réenrichir).
    pub async fn run_action(&self, list_id: &str, element_id: &str, action_id: &str) -> Result<J, String> {
        let (list, item) = self.element_of(list_id, element_id).ok_or("Fiche introuvable.")?;
        let action = js::arr(&list["actions"]).into_iter().find(|a| js::gs(a, "id") == action_id).ok_or("Cette action n'existe pas dans cette liste.")?;
        let o = self.o.up();
        let title = schema::title_of(&list, &item);
        let tpl = js::gs(&action, "gabarit");
        match js::gs(&action, "type").as_str() {
            "url" => {
                let url = shopping::safe_url(&json!(expand_template(&tpl, &list, &item, true))).filter(|u| u.starts_with("https://")).ok_or("Adresse invalide.")?;
                let r = o.local_tool("mcp__system__ouvrir_url", json!({ "url": url }), Box::new(|prm| Box::pin(async move { system::ouvrir_url(&prm).await.unwrap_or_else(|e| json!({ "ok": false, "error": e })) }))).await;
                if r["ok"] == json!(false) {
                    return Ok(json!({ "ok": false, "say": if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "Ouverture refusée.".into() } }));
                }
                Ok(json!({ "ok": true, "say": format!("{} : {title}.", js::gs(&action, "libelle")) }))
            }
            "spotify" => {
                let q = expand_template(&tpl, &list, &item, false);
                let r = o.play_on_spotify(&q).await;
                if js::gb(&r, "ok") {
                    Ok(json!({ "ok": true, "say": format!("Je lance {title} sur Spotify.") }))
                } else {
                    Ok(json!({ "ok": false, "say": if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "Spotify ne répond pas.".into() } }))
                }
            }
            "prix" => {
                self.start_job(&list, &item, "voix", JobKind::Prix, false);
                Ok(json!({ "ok": true, "say": format!("Je cherche le prix actuel de {title}. Je te préviens.") }))
            }
            "enrichir" => self.enrich(list_id, element_id, "voix", EnrichOpts { ecraser: true, suivant: false }).await,
            _ => Err("Type d'action inconnu.".into()),
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn lien_de_boutique_verifie() {
        assert_eq!(shop_url(&json!("https://www.lego.com/fr-fr/product/75192#avis")).as_deref(), Some("https://www.lego.com/fr-fr/product/75192"));
        assert!(shop_url(&json!("http://www.lego.com/fr-fr/product/75192")).is_none(), "https obligatoire");
        assert!(shop_url(&json!("https://192.168.1.10/produit")).is_none(), "adresse locale refusée");
        assert!(shop_url(&json!("javascript:alert(1)")).is_none());
        assert!(shop_url(&json!("")).is_none());
    }

    use super::*;

    fn lego() -> J {
        let mut s = schema::normalize_schema(&schema::preset("lego").unwrap()).unwrap();
        s["id"] = json!("lego_test");
        s["elements"] = json!([]);
        s
    }

    #[test]
    fn requete_musicbrainz() {
        assert_eq!(mb_query("Under a Blood Red Sky", "U2"), "releasegroup:\"Under a Blood Red Sky\" AND artist:\"U2\"");
        assert_eq!(mb_query("Sans \"artiste\"", ""), "releasegroup:\"Sans \\\"artiste\\\"\"");
    }

    #[test]
    fn reponse_musicbrainz() {
        let data = json!({ "release-groups": [
            { "id": "5b6a8f4c-1111-2222-3333-444455556666", "score": 100, "title": "Under a Blood Red Sky", "first-release-date": "1983-11-21",
              "artist-credit": [{ "name": "U2", "joinphrase": "" }], "tags": [{ "name": "rock", "count": 5 }, { "name": "live", "count": 9 }] },
            { "id": "pas-un-identifiant", "score": 99, "title": "Piège" },
            { "id": "00000000-aaaa-bbbb-cccc-dddddddddddd", "score": 80, "title": "Autre", "artist-credit": [{ "name": "A", "joinphrase": " & " }, { "name": "B" }] }
        ] });
        let c = parse_musicbrainz(&data);
        assert_eq!(c.len(), 2, "un identifiant qui n'en est pas un est écarté");
        assert_eq!(c[0]["artiste"], json!("U2"));
        assert_eq!(c[0]["annee"], json!(1983));
        assert_eq!(c[0]["genre"], json!("live"));
        assert_eq!(c[1]["artiste"], json!("A & B"));
    }

    #[test]
    fn titres_proches() {
        assert!(title_close("Under a Blood Red Sky", "under a blood red sky"));
        assert!(title_close("The Joshua Tree", "Joshua Tree"));
        assert!(!title_close("War", "Boy"));
    }

    #[test]
    fn vraies_images_seulement() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0];
        assert_eq!(sniff_image(&jpeg), Some("jpg"));
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Some("png"));
        assert_eq!(sniff_image(b"RIFF\x10\0\0\0WEBPVP8 abcd"), Some("webp"));
        assert_eq!(sniff_image(b"<html><body>Pas une image du tout</body></html>"), None);
        assert_eq!(sniff_image(b"MZ\x90\0\x03\0\0\0 un executable"), None);
    }

    #[test]
    fn adresses_publiques_seulement() {
        assert!(host_is_public("https://images.lego.com/a.jpg"));
        assert!(!host_is_public("http://images.lego.com/a.jpg"));
        assert!(!host_is_public("https://127.0.0.1/a.jpg"));
        assert!(!host_is_public("https://192.168.1.10/a.jpg"));
        assert!(!host_is_public("https://[::1]/a.jpg"));
        assert!(!host_is_public("https://localhost/a.jpg"));
        assert!(!host_is_public("https://imprimante.local/a.jpg"));
        assert!(!host_is_public("https://nas/a.jpg"));
        assert!(!host_is_public("javascript:alert(1)"));
    }

    #[test]
    fn gabarits() {
        let l = lego();
        let item = json!({ "id": "e1", "valeurs": { "numero": "75436", "nom": "Le Faucon & Co" } });
        assert_eq!(expand_template("LEGO {numero} {nom}", &l, &item, false), "LEGO 75436 Le Faucon & Co");
        assert_eq!(expand_template("https://exemple.fr/?q={numero}&n={nom}", &l, &item, true), "https://exemple.fr/?q=75436&n=Le%20Faucon%20%26%20Co");
        assert_eq!(expand_template("{inconnu}x", &l, &item, false), "x");
    }

    #[test]
    fn reponse_web_validee() {
        let l = lego();
        let data = json!({ "valeurs": { "nom": "Faucon Millenium", "pieces": "7 541", "annee": 2017, "etat": "Possédé", "prix_achat": 1, "notes": "pirate", "inconnu": "x", "prix_public": "849,99 €" },
            "image": "https://images.lego.com/x.jpg", "source": "https://www.lego.com/fr-fr/product/75192", "confiance": 3 });
        let f = parse_web_result(&l, &data);
        assert_eq!(f.values.get("nom"), Some(&json!("Faucon Millenium")));
        assert_eq!(f.values.get("pieces"), Some(&json!(7541)));
        assert_eq!(f.values.get("prix_public"), Some(&json!(849.99)));
        for k in ["etat", "prix_achat", "notes", "inconnu"] {
            assert!(!f.values.contains_key(k), "{k} ne doit pas être rempli par le web");
        }
        assert_eq!(f.image.as_deref(), Some("https://images.lego.com/x.jpg"));
        assert_eq!(f.confiance, 1.0);
        let evil = parse_web_result(&l, &json!({ "valeurs": { "nom": "X" }, "image": "https://127.0.0.1/a.png", "source": "javascript:alert(1)" }));
        assert!(evil.image.is_none() && evil.source.is_none());
    }

    #[test]
    fn fusion_sans_ecraser_ta_saisie() {
        let mut vals = Map::new();
        vals.insert("nom".into(), json!("Mon nom à moi"));
        let mut filled = vec![];
        let patch: Map<String, Value> = json!({ "nom": "Nom officiel", "pieces": 7541 }).as_object().unwrap().clone();
        let changed = merge_values(&mut vals, &mut filled, &patch, false, &[]);
        assert_eq!(changed, vec!["pieces".to_string()]);
        assert_eq!(vals["nom"], json!("Mon nom à moi"));
        // Réenrichir : seul ce que l'enrichissement avait rempli est remplacé.
        let patch2: Map<String, Value> = json!({ "nom": "Autre", "pieces": 7600 }).as_object().unwrap().clone();
        merge_values(&mut vals, &mut filled, &patch2, true, &[]);
        assert_eq!(vals["nom"], json!("Mon nom à moi"));
        assert_eq!(vals["pieces"], json!(7600));
        // Titre provisoire (« Set 75436 ») : remplacé par le nom officiel.
        let mut v2 = Map::new();
        v2.insert("nom".into(), json!("Set 75436"));
        merge_values(&mut v2, &mut vec![], &json!({ "nom": "Faucon" }).as_object().unwrap().clone(), false, &["nom".to_string()]);
        assert_eq!(v2["nom"], json!("Faucon"));
    }

    /// Essai réel (réseau) : `cargo test -p jarvis-core enrichissement_reel -- --ignored`.
    #[test]
    #[ignore]
    fn enrichissement_reel_d_un_disque() {
        let dir = std::env::temp_dir().join(format!("jarvis-test-enrich-{}", std::process::id()));
        let svc = Collections::new(std::rc::Weak::<crate::orch::Orchestrator>::new(), dir.clone(), std::rc::Rc::new(|_| {}));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            let r = svc.dispatch("creer_liste", &schema::preset("vinyle").unwrap(), "test").await;
            let lid = js::gs(&r["liste"], "id");
            let a = svc.dispatch("ajouter_element", &json!({ "liste": lid, "valeurs": { "titre": "Under a Blood Red Sky", "artiste": "U2" } }), "test").await;
            let eid = js::gs(&a["element"], "id");
            let e = svc.dispatch("enrichir", &json!({ "liste": lid, "element": eid }), "test").await;
            assert!(js::gb(&e, "ok"), "{e}");
            let list = svc.get(&lid).unwrap();
            let item = js::arr(&list["elements"])[0].clone();
            println!("{}", js::pretty(&schema::public_item(&list, &item)));
            assert_eq!(item["valeurs"]["annee"], json!(1983));
            assert!(js::gs(&item, "image").starts_with("local:"), "pochette copiée : {}", js::gs(&item, "image"));
            let (bytes, ctype) = svc.image_file(&format!("{lid}/{eid}.jpg")).expect("image servie");
            assert_eq!(ctype, "image/jpeg");
            assert!(bytes.len() > 1000);
            // Autre résultat
            let n = svc.dispatch("enrichir", &json!({ "liste": lid, "element": eid, "suivant": true, "ecraser": true }), "test").await;
            println!("{n}");
        });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn consignes_sans_champs_personnels() {
        let l = lego();
        let item = json!({ "id": "e1", "valeurs": { "numero": "75436", "nom": "Set 75436" } });
        let p = info_prompt(&l, &item, "t");
        assert!(p.contains("pieces (nombre)") && p.contains("numero"));
        for k in ["prix_achat (", "lieu_achat", "etat ("] {
            assert!(!p.contains(k), "{k} ne doit pas figurer dans la consigne");
        }
        assert!(p.contains("[liste-info-t]"));
    }
}
