// Format d'un dossier « Savoir » : ce que l'IA écrit (workspace/savoir/<slug>/dossier.json) est relu ici champ
// par champ — types, longueurs, plafonds, adresses publiques seulement — avant d'entrer dans data/savoir/. Rien
// d'autre que ces champs n'est gardé. Un complément (« approfondis le dossier… ») est fusionné sans doublon.

use serde_json::{json, Map, Value};

use super::super::collections::enrich::host_is_public;
use super::super::documents::chunk;
use crate::js::{self, J};

pub const MAX_FICHE: usize = 80;
pub const MAX_SECTIONS: usize = 40;
pub const MAX_CHRONO: usize = 60;
pub const MAX_COMPARAISONS: usize = 10;
pub const MAX_PIECES: usize = 30;
pub const MAX_IMAGES: usize = 60;
pub const MAX_VIDEOS: usize = 20;
pub const MAX_MODELES: usize = 5;
pub const MAX_SOURCES: usize = 80;
pub const MAX_MAQUETTE: usize = 120;
pub const MAX_ATOUTS: usize = 15;
pub const MAX_PRIX: usize = 20;
pub const MAX_AVIS: usize = 15;
pub const MAX_ECOUTE: usize = 30;
pub const ECOUTE_TYPES: &[&str] = &["album", "titre", "concert", "playlist", "bande originale", "autre"];
pub const VUES: &[&str] = &["profil", "face", "dessus", "detail", "cockpit", "interieur", "action", "schema", "autre"];
pub const POSITIONS: &[&str] = &["avant", "arriere", "haut", "bas", "gauche", "droite", "centre"];

/// Texte nettoyé (espaces, longueur).
fn txt(v: &J, max: usize) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let s = crate::re!(r"[ \t\r]+").replace_all(&s, " ");
    let s = crate::re!(r"\n{3,}").replace_all(&s, "\n\n");
    js::slice_chars(s.trim(), max)
}

/// Adresse web publique : https obligatoire pour ce qui est téléchargé (images, modèles 3D), http toléré pour un lien.
pub fn url_ok(url: &str, download: bool) -> bool {
    if download {
        return host_is_public(url);
    }
    if let Some(rest) = url.strip_prefix("http://") {
        return host_is_public(&format!("https://{rest}"));
    }
    host_is_public(url)
}

fn url_of(v: &J, download: bool) -> String {
    let u = txt(v, 600);
    if url_ok(&u, download) { u } else { String::new() }
}

fn items(v: &J, max: usize) -> Vec<&J> {
    v.as_array().map(|a| a.iter().filter(|x| x.is_object()).take(max * 2).collect()).unwrap_or_default()
}

/// Identifiant court : minuscules, chiffres, tirets.
pub fn ident(text: &str, max: usize) -> String {
    let t = js::fold_lower(text);
    let t = crate::re!(r"[^a-z0-9]+").replace_all(&t, "-");
    js::slice_chars(t.trim_matches('-'), max).trim_matches('-').to_string()
}

fn fold_key(s: &str) -> String {
    ident(s, 200)
}

/// Relit un dossier écrit par l'IA. Err si rien d'utilisable.
pub fn validate(raw: &J) -> Result<J, String> {
    if !raw.is_object() {
        return Err("le dossier n'est pas un objet JSON".into());
    }
    let mut fiche: Vec<J> = vec![];
    for f in items(&raw["fiche"], MAX_FICHE) {
        let (cle, valeur) = (txt(&f["cle"], 80), txt(&f["valeur"], 300));
        if cle.is_empty() || valeur.is_empty() || fiche.iter().any(|x| fold_key(&js::gs(x, "cle")) == fold_key(&cle)) {
            continue;
        }
        fiche.push(json!({ "cle": cle, "valeur": valeur, "unite": txt(&f["unite"], 30), "source": url_of(&f["source"], false) }));
    }
    fiche.truncate(MAX_FICHE);

    let mut sections: Vec<J> = vec![];
    for s in items(&raw["sections"], MAX_SECTIONS) {
        let (titre, texte) = (txt(&s["titre"], 120), txt(&s["texte"], 6000));
        if titre.is_empty() || texte.chars().count() < 20 {
            continue;
        }
        let sources: Vec<String> = s["sources"].as_array().map(|a| a.iter().map(|u| url_of(u, false)).filter(|u| !u.is_empty()).take(10).collect()).unwrap_or_default();
        sections.push(json!({ "titre": titre, "texte": texte, "sources": sources }));
    }
    sections.truncate(MAX_SECTIONS);

    let chronologie: Vec<J> = items(&raw["chronologie"], MAX_CHRONO)
        .into_iter()
        .map(|c| json!({ "date": txt(&c["date"], 40), "fait": txt(&c["fait"], 400) }))
        .filter(|c| !js::gs(c, "date").is_empty() && !js::gs(c, "fait").is_empty())
        .take(MAX_CHRONO)
        .collect();

    let comparaisons: Vec<J> = items(&raw["comparaisons"], MAX_COMPARAISONS)
        .into_iter()
        .map(|c| {
            let points: Vec<String> = c["points"].as_array().map(|a| a.iter().map(|p| txt(p, 300)).filter(|p| !p.is_empty()).take(12).collect()).unwrap_or_default();
            json!({ "autre": txt(&c["autre"], 120), "points": points })
        })
        .filter(|c| !js::gs(c, "autre").is_empty() && c["points"].as_array().map(|a| !a.is_empty()).unwrap_or(false))
        .take(MAX_COMPARAISONS)
        .collect();

    let mut pieces: Vec<J> = vec![];
    for p in items(&raw["pieces"], MAX_PIECES) {
        let nom = txt(&p["nom"], 80);
        let id = Some(ident(&txt(&p["id"], 60), 40)).filter(|i| !i.is_empty()).unwrap_or_else(|| ident(&nom, 40));
        if nom.is_empty() || id.is_empty() || pieces.iter().any(|x| js::gs(x, "id") == id) {
            continue;
        }
        let pos = js::fold_lower(&txt(&p["position"], 20));
        pieces.push(json!({ "id": id, "nom": nom, "description": txt(&p["description"], 600), "position": if POSITIONS.contains(&pos.as_str()) { pos } else { "centre".into() } }));
    }
    pieces.truncate(MAX_PIECES);

    let mut images: Vec<J> = vec![];
    for i in items(&raw["images"], MAX_IMAGES) {
        let url = url_of(&i["url"], true);
        if url.is_empty() || images.iter().any(|x| js::gs(x, "url") == url) {
            continue;
        }
        let vue = js::fold_lower(&txt(&i["vue"], 20));
        images.push(json!({
            "url": url, "legende": txt(&i["legende"], 200), "vue": if VUES.contains(&vue.as_str()) { vue } else { "autre".into() },
            "licence": txt(&i["licence"], 80), "source": url_of(&i["source"], false)
        }));
    }
    images.truncate(MAX_IMAGES);

    let mut videos: Vec<J> = vec![];
    for v in items(&raw["videos"], MAX_VIDEOS) {
        let url = url_of(&v["url"], false);
        if url.is_empty() || videos.iter().any(|x| js::gs(x, "url") == url) {
            continue;
        }
        videos.push(json!({ "url": url, "titre": txt(&v["titre"], 160), "duree": txt(&v["duree"], 20), "resume": txt(&v["resume"], 1500) }));
    }
    videos.truncate(MAX_VIDEOS);

    let mut modeles: Vec<J> = vec![];
    for m in items(&raw["modeles3d"], MAX_MODELES) {
        let page = url_of(&m["url_page"], false);
        let glb = url_of(&m["url_glb"], true);
        if page.is_empty() && glb.is_empty() {
            continue;
        }
        modeles.push(json!({ "url_page": page, "url_glb": glb, "licence": txt(&m["licence"], 80), "auteur": txt(&m["auteur"], 80) }));
    }
    modeles.truncate(MAX_MODELES);

    let mut sources: Vec<J> = vec![];
    for s in items(&raw["sources"], MAX_SOURCES) {
        let url = url_of(&s["url"], false);
        if url.is_empty() || sources.iter().any(|x| js::gs(x, "url") == url) {
            continue;
        }
        sources.push(json!({ "titre": Some(txt(&s["titre"], 200)).filter(|t| !t.is_empty()).unwrap_or_else(|| url.clone()), "url": url }));
    }
    sources.truncate(MAX_SOURCES);

    // Champs des profils (produit, musique, film, lieu…) : points forts et faibles, prix, avis, écoute.
    let atouts = |k: &str| -> Vec<J> {
        let mut seen: Vec<String> = vec![];
        raw[k].as_array().into_iter().flatten().map(|x| txt(x, 240)).filter(|x| !x.is_empty() && { let f = fold_key(x); !seen.contains(&f) && { seen.push(f); true } }).take(MAX_ATOUTS).map(|x| json!(x)).collect()
    };
    let prix: Vec<J> = items(&raw["prix"], MAX_PRIX)
        .into_iter()
        .map(|x| {
            let montant = x["prix"].as_f64().or_else(|| txt(&x["prix"], 30).replace(['€', ' ', '\u{a0}'], "").replace(',', ".").parse().ok()).filter(|n| n.is_finite() && *n >= 0.0);
            json!({ "vendeur": txt(&x["vendeur"], 80), "prix": montant, "devise": Some(txt(&x["devise"], 6)).filter(|d| !d.is_empty()).unwrap_or_else(|| "€".into()), "url": url_of(&x["url"], false), "date": txt(&x["date"], 30) })
        })
        .filter(|x| !js::gs(x, "vendeur").is_empty() && !x["prix"].is_null())
        .take(MAX_PRIX)
        .collect();
    let avis: Vec<J> = items(&raw["avis"], MAX_AVIS)
        .into_iter()
        .map(|x| json!({ "source": txt(&x["source"], 80), "note": txt(&x["note"], 12), "sur": txt(&x["sur"], 6), "resume": txt(&x["resume"], 400), "url": url_of(&x["url"], false) }))
        .filter(|x| !js::gs(x, "source").is_empty() && (!js::gs(x, "note").is_empty() || !js::gs(x, "resume").is_empty()))
        .take(MAX_AVIS)
        .collect();
    let ecoute: Vec<J> = items(&raw["ecoute"], MAX_ECOUTE)
        .into_iter()
        .map(|x| {
            let kind = js::fold_lower(&txt(&x["type"], 20));
            let spotify = Some(url_of(&x["spotify"], false)).filter(|u| u.starts_with("https://open.spotify.com/")).unwrap_or_default();
            let youtube = Some(url_of(&x["youtube"], false)).filter(|u| crate::re!(r"^https://(?:www\.|m\.)?(?:youtube\.com/|youtu\.be/)").is_match(u)).unwrap_or_default();
            json!({ "titre": txt(&x["titre"], 160), "type": if ECOUTE_TYPES.contains(&kind.as_str()) { kind } else { "autre".into() }, "artiste": txt(&x["artiste"], 120), "annee": txt(&x["annee"], 10), "spotify": spotify, "youtube": youtube })
        })
        .filter(|x| !js::gs(x, "titre").is_empty())
        .take(MAX_ECOUTE)
        .collect();

    let maquette = maquette(&raw["maquette"]);
    let out = json!({
        "sujet": txt(&raw["sujet"], 120), "resume": txt(&raw["resume"], 2500), "categorie": txt(&raw["categorie"], 60),
        "fiche": fiche, "sections": sections, "chronologie": chronologie, "comparaisons": comparaisons, "pieces": pieces,
        "images": images, "videos": videos, "modeles3d": modeles, "maquette": maquette, "sources": sources,
        "points_forts": atouts("points_forts"), "points_faibles": atouts("points_faibles"), "prix": prix, "avis": avis, "ecoute": ecoute
    });
    if js::gs(&out, "resume").is_empty() && out["sections"].as_array().map(|a| a.is_empty()).unwrap_or(true) && out["fiche"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
        return Err("le dossier ne contient ni résumé, ni section, ni fiche".into());
    }
    Ok(out)
}

/// Maquette 3D procédurale (étape 3) : arbre de primitives, gardé seulement s'il est bien formé.
pub fn maquette(v: &J) -> J {
    const FORMES: &[&str] = &["cylindre", "sphere", "boite", "capsule", "cone", "tore", "profil_extrude"];
    let Some(elements) = v["elements"].as_array() else { return Value::Null };
    let num3 = |x: &J| -> Option<J> {
        let a = x.as_array()?;
        (a.len() == 3 && a.iter().all(|n| n.as_f64().map(|f| f.is_finite() && f.abs() <= 1000.0).unwrap_or(false))).then(|| x.clone())
    };
    let out: Vec<J> = elements
        .iter()
        .filter(|e| FORMES.contains(&js::gs(e, "forme").as_str()))
        .take(MAX_MAQUETTE)
        .map(|e| {
            let mut m = Map::new();
            m.insert("forme".into(), e["forme"].clone());
            for k in ["dimensions", "position", "rotation"] {
                if let Some(v) = num3(&e[k]) {
                    m.insert(k.into(), v);
                }
            }
            let couleur = txt(&e["couleur"], 9);
            if crate::re!(r"^#[0-9a-fA-F]{6}$").is_match(&couleur) {
                m.insert("couleur".into(), json!(couleur));
            }
            let piece = ident(&txt(&e["piece"], 60), 40);
            if !piece.is_empty() {
                m.insert("piece".into(), json!(piece));
            }
            let anime = txt(&e["anime"], 20);
            if ["rotation_x", "rotation_y", "rotation_z"].contains(&anime.as_str()) {
                m.insert("anime".into(), json!(anime));
            }
            Value::Object(m)
        })
        .collect();
    if out.is_empty() {
        Value::Null
    } else {
        json!({ "echelle": txt(&v["echelle"], 40), "elements": out })
    }
}

fn arr<'a>(v: &'a mut J, k: &str) -> &'a mut Vec<J> {
    if !v[k].is_array() {
        v[k] = json!([]);
    }
    v[k].as_array_mut().unwrap()
}

/// (liste, clé d'un élément pour repérer les doublons, plafond)
type MergeRule = (&'static str, fn(&J) -> String, usize);

/// Ajoute à `base` ce que `add` apporte de nouveau. Rend le nombre d'éléments ajoutés.
pub fn merge(base: &mut J, add: &J) -> usize {
    let mut n = 0;
    for k in ["resume", "categorie"] {
        if js::gs(base, k).is_empty() && !js::gs(add, k).is_empty() {
            base[k] = add[k].clone();
            n += 1;
        }
    }
    let rules: &[MergeRule] = &[
        ("fiche", |x| fold_key(&js::gs(x, "cle")), MAX_FICHE),
        ("sections", |x| format!("{}|{}", fold_key(&js::gs(x, "titre")), crate::orch::collections::index::hash(&js::gs(x, "texte"))), MAX_SECTIONS * 2),
        ("chronologie", |x| format!("{}|{}", fold_key(&js::gs(x, "date")), fold_key(&js::gs(x, "fait"))), MAX_CHRONO),
        ("comparaisons", |x| fold_key(&js::gs(x, "autre")), MAX_COMPARAISONS),
        ("pieces", |x| js::gs(x, "id"), MAX_PIECES),
        ("images", |x| js::gs(x, "url"), MAX_IMAGES),
        ("videos", |x| js::gs(x, "url"), MAX_VIDEOS),
        ("modeles3d", |x| format!("{}|{}", js::gs(x, "url_glb"), js::gs(x, "url_page")), MAX_MODELES),
        ("sources", |x| js::gs(x, "url"), MAX_SOURCES * 2),
        ("points_forts", |x| fold_key(&js::s(x)), MAX_ATOUTS),
        ("points_faibles", |x| fold_key(&js::s(x)), MAX_ATOUTS),
        ("prix", |x| fold_key(&js::gs(x, "vendeur")), MAX_PRIX),
        ("avis", |x| fold_key(&js::gs(x, "source")), MAX_AVIS),
        ("ecoute", |x| format!("{}|{}", fold_key(&js::gs(x, "titre")), js::gs(x, "type")), MAX_ECOUTE),
    ];
    for (k, key, max) in rules {
        let incoming = add[*k].as_array().cloned().unwrap_or_default();
        let list = arr(base, k);
        for item in incoming {
            let id = key(&item);
            if list.len() >= *max || list.iter().any(|x| key(x) == id) {
                continue;
            }
            list.push(item);
            n += 1;
        }
    }
    if base["maquette"].is_null() && !add["maquette"].is_null() {
        base["maquette"] = add["maquette"].clone();
        n += 1;
    }
    n
}

/// Textes à indexer : résumé, fiche, sections, chronologie, comparaisons, pièces, images, vidéos, rapports.
pub fn passages_text(d: &J, rapports: &[String]) -> Vec<String> {
    let sujet = js::gs(d, "sujet");
    let mut out: Vec<String> = vec![];
    let resume = js::gs(d, "resume");
    if !resume.is_empty() {
        out.extend(chunk(&format!("{sujet} — résumé"), &resume));
    }
    let fiche: Vec<String> = d["fiche"].as_array().map(|a| a.iter().map(|f| format!("{} : {}{}", js::gs(f, "cle"), js::gs(f, "valeur"), Some(js::gs(f, "unite")).filter(|u| !u.is_empty()).map(|u| format!(" {u}")).unwrap_or_default())).collect()).unwrap_or_default();
    for group in fiche.chunks(12) {
        out.push(format!("« {sujet} — fiche technique » : {}", group.join(" ; ")));
    }
    for s in d["sections"].as_array().into_iter().flatten() {
        out.extend(chunk(&format!("{sujet} — {}", js::gs(s, "titre")), &js::gs(s, "texte")));
    }
    let chrono: Vec<String> = d["chronologie"].as_array().map(|a| a.iter().map(|c| format!("{} : {}", js::gs(c, "date"), js::gs(c, "fait"))).collect()).unwrap_or_default();
    for group in chrono.chunks(10) {
        out.push(format!("« {sujet} — chronologie » : {}", group.join(" ; ")));
    }
    for c in d["comparaisons"].as_array().into_iter().flatten() {
        out.push(format!("« {sujet} comparé à {} » : {}", js::gs(c, "autre"), js::str_list(&c["points"]).join(" ; ")));
    }
    for p in d["pieces"].as_array().into_iter().flatten() {
        out.push(format!("« {sujet} — {} » ({}) : {}", js::gs(p, "nom"), js::gs(p, "position"), js::gs(p, "description")));
    }
    let legendes: Vec<String> = d["images"].as_array().map(|a| a.iter().map(|i| js::gs(i, "legende")).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
    if !legendes.is_empty() {
        out.push(format!("« {sujet} — images » : {}", legendes.join(" ; ")));
    }
    for v in d["videos"].as_array().into_iter().flatten() {
        let resume = js::gs(v, "resume");
        out.push(format!("« {sujet} — vidéo {} » : {}", js::gs(v, "titre"), if resume.is_empty() { js::gs(v, "url") } else { resume }));
    }
    for (k, label) in [("points_forts", "points forts"), ("points_faibles", "points faibles")] {
        let list = js::str_list(&d[k]);
        if !list.is_empty() {
            out.push(format!("« {sujet} — {label} » : {}", list.join(" ; ")));
        }
    }
    let prix: Vec<String> = js::arr(&d["prix"]).iter().map(|p| format!("{} : {} {}{}", js::gs(p, "vendeur"), js::s(&p["prix"]), js::gs(p, "devise"), Some(js::gs(p, "date")).filter(|x| !x.is_empty()).map(|x| format!(" ({x})")).unwrap_or_default())).collect();
    if !prix.is_empty() {
        out.push(format!("« {sujet} — prix » : {}", prix.join(" ; ")));
    }
    for a in js::arr(&d["avis"]) {
        let note = Some(js::gs(&a, "note")).filter(|n| !n.is_empty()).map(|n| format!(" {n}{}", Some(js::gs(&a, "sur")).filter(|s| !s.is_empty()).map(|s| format!("/{s}")).unwrap_or_default())).unwrap_or_default();
        out.push(format!("« {sujet} — avis de {} »{note} : {}", js::gs(&a, "source"), js::gs(&a, "resume")));
    }
    let ecoute: Vec<String> = js::arr(&d["ecoute"]).iter().map(|e| format!("{} ({}{}{})", js::gs(e, "titre"), js::gs(e, "type"), Some(js::gs(e, "artiste")).filter(|x| !x.is_empty()).map(|x| format!(", {x}")).unwrap_or_default(), Some(js::gs(e, "annee")).filter(|x| !x.is_empty()).map(|x| format!(", {x}")).unwrap_or_default())).collect();
    for group in ecoute.chunks(12) {
        out.push(format!("« {sujet} — à écouter » : {}", group.join(" ; ")));
    }
    for r in rapports {
        out.extend(chunk(&format!("{sujet} — rapport"), r));
    }
    out.retain(|p| p.chars().count() >= 20);
    out
}

/// Nombre d'éléments par rubrique (card de suivi, statistiques).
pub fn counts(d: &J) -> J {
    let n = |k: &str| d[k].as_array().map(|a| a.len()).unwrap_or(0);
    json!({
        "sources": n("sources"), "fiche": n("fiche"), "sections": n("sections"), "chronologie": n("chronologie"), "comparaisons": n("comparaisons"),
        "pieces": n("pieces"), "images": n("images"), "videos": n("videos"), "modeles": n("modeles3d"), "maquette": !d["maquette"].is_null(),
        "resume": !js::gs(d, "resume").is_empty(), "atouts": n("points_forts") + n("points_faibles"), "prix": n("prix"), "avis": n("avis"), "ecoute": n("ecoute")
    })
}

/// Éléments d'un dossier, un par ligne : (clé unique, rubrique, texte lisible). Sert à repérer ce qui vient d'arriver.
pub fn items_of(d: &J) -> Vec<(String, &'static str, String)> {
    let mut out = vec![];
    let list = |k: &str| d[k].as_array().cloned().unwrap_or_default();
    if !js::gs(d, "resume").is_empty() {
        out.push(("resume".into(), "résumé", js::slice_chars(&js::gs(d, "resume"), 160)));
    }
    for f in list("fiche") {
        let unit = Some(js::gs(&f, "unite")).filter(|u| !u.is_empty()).map(|u| format!(" {u}")).unwrap_or_default();
        out.push((format!("f:{}", fold_key(&js::gs(&f, "cle"))), "fiche", format!("{} : {}{unit}", js::gs(&f, "cle"), js::gs(&f, "valeur"))));
    }
    for s in list("sections") {
        out.push((format!("s:{}", fold_key(&js::gs(&s, "titre"))), "section", js::gs(&s, "titre")));
    }
    for c in list("chronologie") {
        out.push((format!("c:{}:{}", fold_key(&js::gs(&c, "date")), fold_key(&js::gs(&c, "fait"))), "chronologie", format!("{} — {}", js::gs(&c, "date"), js::gs(&c, "fait"))));
    }
    for c in list("comparaisons") {
        out.push((format!("v:{}", fold_key(&js::gs(&c, "autre"))), "comparaison", format!("face à {}", js::gs(&c, "autre"))));
    }
    for p in list("pieces") {
        out.push((format!("p:{}", js::gs(&p, "id")), "pièce", js::gs(&p, "nom")));
    }
    for i in list("images") {
        let what = Some(js::gs(&i, "legende")).filter(|l| !l.is_empty()).unwrap_or_else(|| js::gs(&i, "url"));
        out.push((format!("i:{}", js::gs(&i, "url")), "image", format!("{} ({})", what, js::gs(&i, "vue"))));
    }
    for v in list("videos") {
        out.push((format!("y:{}", js::gs(&v, "url")), "vidéo", Some(js::gs(&v, "titre")).filter(|t| !t.is_empty()).unwrap_or_else(|| js::gs(&v, "url"))));
    }
    for m in list("modeles3d") {
        out.push((format!("m:{}{}", js::gs(&m, "url_glb"), js::gs(&m, "url_page")), "modèle 3D", Some(js::gs(&m, "url_page")).filter(|t| !t.is_empty()).unwrap_or_else(|| js::gs(&m, "url_glb"))));
    }
    if !d["maquette"].is_null() {
        out.push(("maquette".into(), "maquette 3D", format!("{} formes", d["maquette"]["elements"].as_array().map(|a| a.len()).unwrap_or(0))));
    }
    for (k, label) in [("points_forts", "point fort"), ("points_faibles", "point faible")] {
        for x in js::str_list(&d[k]) {
            out.push((format!("{k}:{}", fold_key(&x)), label, x));
        }
    }
    for p in list("prix") {
        out.push((format!("€:{}", fold_key(&js::gs(&p, "vendeur"))), "prix", format!("{} : {} {}", js::gs(&p, "vendeur"), js::s(&p["prix"]), js::gs(&p, "devise"))));
    }
    for a in list("avis") {
        out.push((format!("a:{}", fold_key(&js::gs(&a, "source"))), "avis", format!("{} {}", js::gs(&a, "source"), js::gs(&a, "note"))));
    }
    for e in list("ecoute") {
        out.push((format!("e:{}|{}", fold_key(&js::gs(&e, "titre")), js::gs(&e, "type")), "écoute", js::gs(&e, "titre")));
    }
    for s in list("sources") {
        out.push((format!("u:{}", js::gs(&s, "url")), "source", js::gs(&s, "titre")));
    }
    out
}

/// JSON lu dans un fichier écrit par l'IA : tolère un bloc ```json ou du texte autour de l'objet.
pub fn parse_loose(text: &str) -> Option<J> {
    if let Ok(v) = serde_json::from_str::<J>(text.trim()) {
        return Some(v);
    }
    let (a, b) = (text.find('{')?, text.rfind('}')?);
    (b > a).then(|| serde_json::from_str::<J>(&text[a..=b]).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> J {
        json!({
            "sujet": "Airbus H160", "resume": "Hélicoptère bimoteur moyen d'Airbus Helicopters, certifié en 2020.", "categorie": "hélicoptère",
            "fiche": [{ "cle": "Vitesse de croisière", "valeur": "287", "unite": "km/h", "source": "https://www.airbus.com/h160" },
                      { "cle": "vitesse de croisière", "valeur": "doublon" }, { "cle": "Moteurs", "valeur": 2 }],
            "sections": [{ "titre": "Conception", "texte": "Le H160 utilise des pales Blue Edge pour réduire le bruit.", "sources": ["https://fr.wikipedia.org/wiki/H160", "javascript:alert(1)"] },
                         { "titre": "Vide", "texte": "court" }],
            "pieces": [{ "id": "Rotor principal", "nom": "Rotor principal", "description": "Cinq pales Blue Edge", "position": "haut" },
                       { "nom": "Fenestron", "position": "nulle-part" }],
            "images": [{ "url": "https://upload.wikimedia.org/a.jpg", "vue": "Profil", "legende": "H160 en vol" },
                       { "url": "http://exemple.com/b.jpg" }, { "url": "https://192.168.1.10/c.jpg" }, { "url": "https://localhost/d.png" }],
            "videos": [{ "url": "https://www.youtube.com/watch?v=abc", "titre": "Présentation" }],
            "modeles3d": [{ "url_glb": "http://exemple.com/m.glb" }, { "url_page": "https://sketchfab.com/3d-models/h160", "licence": "CC-BY" }],
            "sources": [{ "titre": "Airbus", "url": "https://www.airbus.com/h160" }, { "url": "file:///C:/secret.txt" }],
            "maquette": { "elements": [{ "forme": "cylindre", "dimensions": [1, 2, 1], "position": [0, 0, 0], "couleur": "#ff0000", "anime": "rotation_y", "piece": "Rotor principal" },
                                       { "forme": "script", "code": "alert(1)" }] },
            "inconnu": "ignoré"
        })
    }

    #[test]
    fn un_dossier_est_relu_champ_par_champ() {
        let d = validate(&sample()).unwrap();
        assert_eq!(d["fiche"].as_array().unwrap().len(), 2, "doublon de clé écarté");
        assert_eq!(d["fiche"][1]["valeur"], "2");
        assert_eq!(d["sections"].as_array().unwrap().len(), 1);
        assert_eq!(d["sections"][0]["sources"].as_array().unwrap().len(), 1, "adresse javascript: refusée");
        assert_eq!(d["pieces"][0]["id"], "rotor-principal");
        assert_eq!(d["pieces"][1]["position"], "centre");
        assert_eq!(d["images"].as_array().unwrap().len(), 1, "http, IP privée et localhost refusés pour un téléchargement");
        assert_eq!(d["images"][0]["vue"], "profil");
        assert_eq!(d["modeles3d"].as_array().unwrap().len(), 1);
        assert_eq!(d["sources"].as_array().unwrap().len(), 1);
        assert_eq!(d["maquette"]["elements"].as_array().unwrap().len(), 1, "forme inconnue écartée");
        assert_eq!(d["maquette"]["elements"][0]["piece"], "rotor-principal");
        assert!(d.get("inconnu").is_none());
        assert!(validate(&json!({ "sujet": "vide" })).is_err());
        assert!(validate(&json!([1, 2])).is_err());
    }

    #[test]
    fn un_complement_est_fusionne_sans_doublon() {
        let mut base = validate(&sample()).unwrap();
        let add = validate(&json!({
            "resume": "autre résumé",
            "fiche": [{ "cle": "Vitesse de croisière", "valeur": "290" }, { "cle": "Masse maximale", "valeur": "6050", "unite": "kg" }],
            "sources": [{ "url": "https://www.airbus.com/h160" }, { "url": "https://www.aerobuzz.fr/h160" }]
        }))
        .unwrap();
        let n = merge(&mut base, &add);
        assert_eq!(n, 2);
        assert_eq!(base["fiche"].as_array().unwrap().len(), 3);
        assert_eq!(base["fiche"][0]["valeur"], "287", "une valeur connue n'est pas écrasée");
        assert_eq!(base["sources"].as_array().unwrap().len(), 2);
        assert!(js::gs(&base, "resume").starts_with("Hélicoptère"));
    }

    #[test]
    fn passages_et_json_approximatif() {
        let d = validate(&sample()).unwrap();
        let p = passages_text(&d, &["# Rapport\n\nLe H160 remplace le Dauphin dans de nombreuses flottes.".into()]);
        assert!(p.iter().any(|t| t.contains("fiche technique") && t.contains("287 km/h")));
        assert!(p.iter().any(|t| t.contains("Rotor principal")));
        assert!(p.iter().any(|t| t.contains("Dauphin")));
        assert_eq!(parse_loose("Voici :\n```json\n{\"a\": 1}\n```").unwrap()["a"], 1);
        assert!(parse_loose("rien").is_none());
        assert_eq!(ident("L'hélicoptère Airbus H160 !", 50), "l-helicoptere-airbus-h160");
        let c = counts(&d);
        assert_eq!((c["fiche"].as_u64(), c["images"].as_u64(), c["maquette"].as_bool()), (Some(2), Some(1), Some(true)));
        let items = items_of(&d);
        assert!(items.iter().any(|(_, k, t)| *k == "fiche" && t == "Vitesse de croisière : 287 km/h"));
        let keys: std::collections::HashSet<_> = items.iter().map(|(k, _, _)| k.clone()).collect();
        assert_eq!(keys.len(), items.len(), "chaque élément a une clé unique (repérage des nouveautés)");
    }
}
