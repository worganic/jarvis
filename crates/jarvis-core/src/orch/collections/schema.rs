// Schéma des listes à fiches (vinyles, Lego…) : modèles livrés, validation des champs et des valeurs,
// lecture d'un élément (titre, statut, passage du RAG). Tout ce qui entre dans une liste — saisie à la
// voix, formulaire de la card, formulaire de la page d'aide, réponse de l'IA — passe par ces fonctions :
// listes blanches, plafonds de longueur, jamais de code ni d'adresse libre.

use std::sync::LazyLock;

use serde_json::{json, Value};

use crate::js::{self, J};

static PRESETS: LazyLock<Vec<J>> = LazyLock::new(|| serde_json::from_str(include_str!("../../../data/collection-presets.json")).unwrap_or_default());

pub const FIELD_TYPES: &[&str] = &["texte", "texte_long", "nombre", "prix", "date", "choix", "oui_non", "lien", "image"];
pub const ACTION_TYPES: &[&str] = &["spotify", "url", "prix", "enrichir"];
pub const PROVIDERS: &[&str] = &["disque", "serie", "web", "aucun"];
pub const MAX_FIELDS: usize = 14;
pub const MAX_ITEMS: usize = 5000;
pub const MAX_LISTS: usize = 60;

pub fn presets() -> &'static [J] {
    &PRESETS
}

pub fn preset(id: &str) -> Option<J> {
    PRESETS.iter().find(|p| js::gs(p, "id") == id).cloned()
}

/// Minuscules sans accents, mots séparés par une espace.
pub fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

/// Clé technique : « Date d'achat » → « date_d_achat ».
pub fn slug(s: &str) -> String {
    fold(s).replace(' ', "_")
}

/// Texte sur une ligne, sans caractères de contrôle, plafonné.
pub fn clean_text(s: &str, max: usize) -> String {
    let s = crate::re!(r"[\x00-\x1f\x7f]+").replace_all(s, " ");
    js::slice_chars(crate::re!(r"\s+").replace_all(&s, " ").trim(), max)
}

/// Texte long : les retours à la ligne sont conservés.
pub fn clean_long(s: &str, max: usize) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let s = crate::re!(r"[\x00-\x08\x0b-\x1f\x7f]+").replace_all(&s, " ");
    js::slice_chars(s.trim(), max)
}

/// Le modèle livré dont un des mots-clés figure dans la phrase (texte brut ou replié).
pub fn detect_preset(text: &str) -> Option<J> {
    let padded = format!(" {} ", fold(text));
    PRESETS.iter().find(|p| js::str_list(&p["motsCles"]).iter().any(|k| padded.contains(&format!(" {} ", fold(k))))).cloned()
}

// --- Champs --------------------------------------------------------------------------------------

pub fn normalize_field(f: &J) -> Option<J> {
    let label = clean_text(&js::gs(f, "libelle"), 40);
    let label = if label.is_empty() { clean_text(&js::gs(f, "cle"), 40) } else { label };
    if label.is_empty() {
        return None;
    }
    let mut key = slug(&js::gs(f, "cle"));
    if key.is_empty() {
        key = slug(&label);
    }
    let key = js::slice_chars(&key, 30);
    if key.is_empty() {
        return None;
    }
    let mut typ = js::gs(f, "type");
    if !FIELD_TYPES.contains(&typ.as_str()) {
        typ = "texte".into();
    }
    let mut out = json!({ "cle": key, "libelle": label, "type": typ });
    if typ == "choix" {
        let mut choices: Vec<String> = vec![];
        for c in js::str_list(&f["choix"]) {
            let c = clean_text(&c, 40);
            if !c.is_empty() && !choices.iter().any(|x| fold(x) == fold(&c)) && choices.len() < 12 {
                choices.push(c);
            }
        }
        if choices.is_empty() {
            out["type"] = json!("texte");
        } else {
            out["choix"] = json!(choices);
        }
    }
    let help = clean_text(&js::gs(f, "aide"), 80);
    if !help.is_empty() {
        out["aide"] = json!(help);
    }
    // Champ obligatoire : une fiche ne s'enregistre pas sans lui (ex. date d'un concert, pour trier par date).
    if js::gb(f, "obligatoire") {
        out["obligatoire"] = json!(true);
    }
    Some(out)
}

/// Champs normalisés, clés uniques, 14 au plus.
pub fn normalize_fields(list: &J) -> Vec<J> {
    let mut out: Vec<J> = vec![];
    for f in js::arr(list) {
        if out.len() >= MAX_FIELDS {
            break;
        }
        let Some(mut f) = normalize_field(&f) else { continue };
        let base = js::gs(&f, "cle");
        let mut key = base.clone();
        let mut n = 2;
        while out.iter().any(|o| js::gs(o, "cle") == key) {
            key = format!("{base}_{n}");
            n += 1;
        }
        f["cle"] = json!(key);
        out.push(f);
    }
    out
}

pub fn field<'a>(list: &'a J, key: &str) -> Option<&'a J> {
    list["champs"].as_array()?.iter().find(|f| js::gs(f, "cle") == key)
}

fn keys_of(fields: &[J]) -> Vec<String> {
    fields.iter().map(|f| js::gs(f, "cle")).collect()
}

/// Les `{champ}` d'un gabarit ; None si le gabarit contient autre chose qu'un champ connu entre accolades.
pub fn placeholders(template: &str, keys: &[String]) -> Option<Vec<String>> {
    let re = crate::re!(r"\{([a-z0-9_]+)\}");
    let mut found = vec![];
    for m in re.captures_iter(template) {
        let k = m[1].to_string();
        if !keys.contains(&k) {
            return None;
        }
        found.push(k);
    }
    let rest = re.replace_all(template, "");
    if rest.contains('{') || rest.contains('}') {
        return None;
    }
    Some(found)
}

pub fn normalize_actions(actions: &J, keys: &[String]) -> Vec<J> {
    let mut out: Vec<J> = vec![];
    for a in js::arr(actions) {
        if out.len() >= 4 {
            break;
        }
        let typ = js::gs(&a, "type");
        if !ACTION_TYPES.contains(&typ.as_str()) {
            continue;
        }
        let label = clean_text(&js::gs(&a, "libelle"), 30);
        let template = clean_text(&js::gs(&a, "gabarit"), 200);
        if label.is_empty() || placeholders(&template, keys).is_none() {
            continue;
        }
        if typ == "url" && !template.starts_with("https://") {
            continue;
        }
        if typ != "enrichir" && template.is_empty() {
            continue;
        }
        let mut id = slug(&js::gs(&a, "id"));
        if id.is_empty() {
            id = slug(&label);
        }
        let id = js::slice_chars(&id, 20);
        if out.iter().any(|o| js::gs(o, "id") == id) {
            continue;
        }
        let icon = js::slice_chars(js::gs(&a, "icone").trim(), 4);
        out.push(json!({ "id": id, "libelle": label, "icone": if icon.is_empty() { "↗".to_string() } else { icon }, "type": typ, "gabarit": template }));
    }
    out
}

fn existing_keys(v: &J, keys: &[String], max: usize, exclude: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for k in js::str_list(v) {
        if keys.contains(&k) && k != exclude && !out.contains(&k) && out.len() < max {
            out.push(k);
        }
    }
    out
}

/// Schéma complet (sans identifiant, dates ni éléments) à partir d'une saisie ou d'un modèle.
pub fn normalize_schema(input: &J) -> Result<J, String> {
    let name = clean_text(&js::gs(input, "nom"), 60);
    if name.is_empty() {
        return Err("Il manque le nom de la liste.".into());
    }
    let fields = normalize_fields(&input["champs"]);
    if fields.is_empty() {
        return Err("Il faut au moins un champ.".into());
    }
    let keys = keys_of(&fields);
    let icon = js::slice_chars(js::gs(input, "icone").trim(), 4);
    let singular = clean_text(&js::gs(&input["noms"], "singulier"), 30);
    let plural = clean_text(&js::gs(&input["noms"], "pluriel"), 30);
    let singular = if singular.is_empty() { "élément".to_string() } else { singular };
    let plural = if plural.is_empty() { format!("{singular}s") } else { plural };
    let mut aliases: Vec<String> = vec![];
    for a in js::str_list(&input["alias"]).into_iter().chain([name.clone(), singular.clone(), plural.clone()]) {
        let a = clean_text(&a, 40).to_lowercase();
        if !a.is_empty() && !aliases.iter().any(|x| fold(x) == fold(&a)) && aliases.len() < 14 {
            aliases.push(a);
        }
    }
    let title = {
        let t = js::gs(input, "titre");
        if keys.contains(&t) {
            t
        } else {
            fields.iter().find(|f| js::gs(f, "type") == "texte").or(fields.first()).map(|f| js::gs(f, "cle")).unwrap_or_default()
        }
    };
    let status = {
        let s = js::gs(input, "etat");
        let ok = |k: &str| field_in(&fields, k).map(|f| js::gs(f, "type") == "choix").unwrap_or(false);
        if ok(&s) {
            Value::String(s)
        } else if ok("etat") {
            json!("etat")
        } else {
            Value::Null
        }
    };
    let image = {
        let i = js::gs(input, "image");
        if field_in(&fields, &i).map(|f| js::gs(f, "type") == "image").unwrap_or(false) {
            Value::String(i)
        } else {
            fields.iter().find(|f| js::gs(f, "type") == "image").map(|f| json!(js::gs(f, "cle"))).unwrap_or(Value::Null)
        }
    };
    let provider = {
        let p = js::gs(&input["enrichissement"], "fournisseur");
        if PROVIDERS.contains(&p.as_str()) { p } else { "aucun".to_string() }
    };
    let vocabulary = normalize_vocab(&input["vocabulaire"], &keys);
    let model = {
        let m = slug(&js::gs(input, "modele"));
        if m.is_empty() { "libre".to_string() } else { js::slice_chars(&m, 20) }
    };
    let sort = {
        let t = js::gs(input, "tri");
        if SORTS.contains(&t.as_str()) { t } else { "recent".to_string() }
    };
    Ok(json!({
        "nom": name,
        "icone": if icon.is_empty() { "📋".to_string() } else { icon },
        "noms": { "singulier": singular, "pluriel": plural },
        "alias": aliases,
        "modele": model,
        "champs": fields,
        "titre": title,
        "sousTitre": existing_keys(&input["sousTitre"], &keys, 3, &title),
        "identifiant": existing_keys(&input["identifiant"], &keys, 2, ""),
        "etat": status,
        "vignette": existing_keys(&input["vignette"], &keys, 2, &title),
        "image": image,
        "actions": normalize_actions(&input["actions"], &keys),
        "enrichissement": { "fournisseur": provider, "consigne": clean_text(&js::gs(&input["enrichissement"], "consigne"), 300) },
        "vocabulaire": vocabulary,
        "tri": sort
    }))
}

/// Les mots du statut d'une liste : « Possédé / Souhaité » par défaut, « Vu / À voir » pour un film, un concert…
/// Ordres d'affichage proposés pour une liste.
/// Ordres d'affichage : date_desc / date_asc trient sur le premier champ de type date (plus récent ou plus ancien d'abord).
pub const SORTS: [&str; 5] = ["recent", "az", "statut", "date_desc", "date_asc"];

pub fn default_vocab() -> J {
    json!({ "fait": "Possédé", "voulu": "Souhaité", "faitPluriel": "Possédés", "vouluPluriel": "Souhaités",
            "ouiPhrase": "Oui, tu l'as", "vouluPhrase": "est dans tes souhaits", "compteFait": "Tu as", "dateFait": "date_achat", "defaut": "fait" })
}

fn normalize_vocab(input: &J, keys: &[String]) -> J {
    let mut out = default_vocab();
    for k in ["fait", "voulu", "faitPluriel", "vouluPluriel", "ouiPhrase", "vouluPhrase", "compteFait"] {
        let v = clean_text(&js::gs(input, k), 60);
        if !v.is_empty() {
            out[k] = json!(v);
        }
    }
    let default = js::gs(input, "defaut");
    if ["fait", "voulu", "date"].contains(&default.as_str()) {
        out["defaut"] = json!(default);
    }
    let wanted = js::gs(input, "dateFait");
    out["dateFait"] = if keys.contains(&wanted) {
        json!(wanted)
    } else if keys.iter().any(|k| k == "date_achat") && (input.is_null() || input["dateFait"].is_null()) {
        json!("date_achat")
    } else {
        Value::Null
    };
    out
}

/// Une fiche ajoutée sans précision est-elle « faite » (possédée, vue) ? Selon la liste : toujours, jamais,
/// ou d'après sa date (un concert passé est vu, un concert à venir est à voir).
pub fn default_is_done(list: &J, values: &serde_json::Map<String, Value>) -> bool {
    match js::gs(&vocab(list), "defaut").as_str() {
        "voulu" => false,
        "date" => match values.get("date").and_then(|d| chrono::NaiveDate::parse_from_str(&js::s(d), "%Y-%m-%d").ok()) {
            Some(d) => d <= js::local_now().date_naive(),
            None => true,
        },
        _ => true,
    }
}

/// Vocabulaire de la liste (celui par défaut si elle n'en a pas : listes créées avant les films, concerts…).
pub fn vocab(list: &J) -> J {
    let mut v = default_vocab();
    if let Some(o) = list["vocabulaire"].as_object() {
        for (k, x) in o {
            v[k] = x.clone();
        }
    } else if js::get(list, "champs").is_array() {
        v["dateFait"] = if field(list, "date_achat").is_some() { json!("date_achat") } else { Value::Null };
    }
    v
}

fn field_in<'a>(fields: &'a [J], key: &str) -> Option<&'a J> {
    fields.iter().find(|f| js::gs(f, "cle") == key)
}

// --- Valeurs -------------------------------------------------------------------------------------

/// « 12,5 » / « 1 983 » / 1983 → nombre.
pub fn parse_number(v: &J) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => {
            let t: String = s.chars().filter(|c| !c.is_whitespace() && *c != '\u{a0}').collect();
            let t = t.replace(',', ".");
            t.parse::<f64>().ok().filter(|f| f.is_finite())
        }
        _ => None,
    }
}

pub fn french_month(name: &str) -> Option<u32> {
    let f = fold(name);
    Some(match f.as_str() {
        "janvier" | "janv" | "jan" => 1,
        "fevrier" | "fevr" | "fev" => 2,
        "mars" | "mar" => 3,
        "avril" | "avr" => 4,
        "mai" => 5,
        "juin" => 6,
        "juillet" | "juil" | "jul" => 7,
        "aout" | "aou" => 8,
        "septembre" | "sept" | "sep" => 9,
        "octobre" | "oct" => 10,
        "novembre" | "nov" => 11,
        "decembre" | "dec" => 12,
        _ => return None,
    })
}

/// Date « 2024-03-12 », « 12/03/2024 », « 27 août 1988 », « 3, 4 et 6 juil. 1990 », « 1993 » → ISO.
pub fn parse_date(s: &str) -> Option<String> {
    let t = s.trim();
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some(d.format("%Y-%m-%d").to_string());
    }
    if let Some(m) = crate::re!(r"^(\d{1,2})[/\-.](\d{1,2})[/\-.](\d{2}|\d{4})$").captures(t) {
        let (d, mo): (u32, u32) = (m[1].parse().ok()?, m[2].parse().ok()?);
        let mut y: i32 = m[3].parse().ok()?;
        if m[3].len() == 2 {
            y += if y > 69 { 1900 } else { 2000 };
        }
        if let Some(dt) = chrono::NaiveDate::from_ymd_opt(y, mo, d) {
            return Some(dt.format("%Y-%m-%d").to_string());
        }
    }
    // Dates textuelles françaises : « 27 août 1988 », « 1er déc. 2014 », « 25 et 26 juil. 2017 », « 13 sept. 1992 »
    if let Some(m) = crate::re!(r"(?i)(?:^|\b)(\d{1,2})(?:er)?(?:\s*(?:,|et|-)\s*\d{1,2})*\s+([a-zA-Zàâäéèêëîïôöùûüç.]+)\s+(\d{4})\b").captures(t) {
        let d: u32 = m[1].parse().ok()?;
        let mo = french_month(&m[2])?;
        let y: i32 = m[3].parse().ok()?;
        if let Some(dt) = chrono::NaiveDate::from_ymd_opt(y, mo, d) {
            return Some(dt.format("%Y-%m-%d").to_string());
        }
    }
    if crate::re!(r"\d{1,2}[/\-.]\d{1,2}[/\-.]").is_match(t) {
        return None;
    }
    // Année seule (ex: « 1993 (estimé) » ou « 1993 ») -> 1er janvier de cette année
    if let Some(m) = crate::re!(r"\b(19\d{2}|20\d{2})\b").captures(t) {
        let y: i32 = m[1].parse().ok()?;
        if let Some(dt) = chrono::NaiveDate::from_ymd_opt(y, 1, 1) {
            return Some(dt.format("%Y-%m-%d").to_string());
        }
    }
    None
}

pub fn date_fr(iso: &str) -> String {
    match chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d") {
        Ok(d) => d.format("%d/%m/%Y").to_string(),
        Err(_) => iso.to_string(),
    }
}

/// Aligne le modèle d'une liste demandée sur ce que propose le site Nomade : statut « Souhaité / Possédé », une image par fiche,
/// recherche IA des informations. Rend true si le modèle a changé.
pub fn align_with_site(list: &mut J) -> bool {
    let mut changed = false;
    let wanted = json!(["Souhaité", "Possédé"]);
    let etat = match list["etat"].as_str() {
        Some(k) if field(list, k).is_some() => k.to_string(),
        _ => "etat".to_string(),
    };
    let mut fields = js::arr(&list["champs"]);
    if let Some(i) = fields.iter().position(|f| js::gs(f, "cle") == etat) {
        if js::gs(&fields[i], "type") != "choix" || fields[i]["choix"] != wanted {
            fields[i]["type"] = json!("choix");
            fields[i]["choix"] = wanted.clone();
            changed = true;
        }
    } else if fields.len() < MAX_FIELDS {
        fields.push(json!({ "cle": etat, "libelle": "État", "type": "choix", "choix": wanted }));
        changed = true;
    }
    if list["etat"].as_str() != Some(etat.as_str()) {
        list["etat"] = json!(etat);
        changed = true;
    }
    if !list["image"].is_string() {
        let key = fields.iter().find(|f| js::gs(f, "type") == "image").map(|f| js::gs(f, "cle"));
        let key = match key {
            Some(k) => Some(k),
            None if fields.len() < MAX_FIELDS && !fields.iter().any(|f| js::gs(f, "cle") == "image") => {
                fields.push(json!({ "cle": "image", "libelle": "Image", "type": "image" }));
                Some("image".to_string())
            }
            None => None,
        };
        if let Some(k) = key {
            list["image"] = json!(k);
            changed = true;
        }
    }
    list["champs"] = json!(fields);
    if js::gs(&list["enrichissement"], "fournisseur") == "aucun" || list["enrichissement"].is_null() {
        list["enrichissement"] = json!({ "fournisseur": "web", "consigne": js::gs(&list["enrichissement"], "consigne") });
        changed = true;
    }
    if list["vocabulaire"].is_null() {
        let mut v = default_vocab();
        v["defaut"] = json!("voulu");
        list["vocabulaire"] = v;
        changed = true;
    }
    changed
}

/// Texte qui veut dire « pas de valeur » : vide, ou le « null » / « N/A » que l'IA ou un formulaire écrit à la place.
pub fn is_blank_text(s: &str) -> bool {
    matches!(fold(s.trim()).as_str(), "" | "null" | "undefined" | "nan" | "n a" | "non renseigne")
}

/// Valeur d'un champ validée : Null pour « vide », Err pour une valeur inacceptable.
pub fn normalize_value(field: &J, v: &J) -> Result<J, String> {
    if v.is_null() || (v.is_string() && is_blank_text(&js::s(v))) {
        return Ok(Value::Null);
    }
    let label = js::gs(field, "libelle");
    match js::gs(field, "type").as_str() {
        "texte" => Ok(json!(clean_text(&js::s(v), 200))),
        "texte_long" => Ok(json!(clean_long(&js::s(v), 2000))),
        "nombre" => {
            let n = parse_number(v).filter(|n| n.abs() < 1e12).ok_or_else(|| format!("« {label} » doit être un nombre."))?;
            Ok(if n.fract() == 0.0 { json!(n as i64) } else { json!(js::round2(n)) })
        }
        "prix" => {
            let p = super::super::shopping::parse_price(v).or_else(|| parse_number(v).filter(|n| (0.0..1e7).contains(n)).map(js::round2));
            p.map(|p| json!(p)).ok_or_else(|| format!("« {label} » doit être un prix."))
        }
        "date" => parse_date(&js::s(v)).map(|d| json!(d)).ok_or_else(|| format!("« {label} » doit être une date (jour/mois/année).")),
        "choix" => {
            let wanted = fold(&js::s(v));
            let choices = js::str_list(&field["choix"]);
            choices
                .iter()
                .find(|c| fold(c) == wanted)
                .or_else(|| choices.iter().find(|c| !wanted.is_empty() && (fold(c).starts_with(&wanted) || wanted.starts_with(&fold(c)))))
                .map(|c| json!(c))
                .ok_or_else(|| format!("« {label} » : choisis parmi {}.", choices.join(", ")))
        }
        "oui_non" => match v {
            Value::Bool(b) => Ok(json!(b)),
            _ => match fold(&js::s(v)).as_str() {
                "oui" | "true" | "1" | "vrai" => Ok(json!(true)),
                "non" | "false" | "0" | "faux" => Ok(json!(false)),
                _ => Err(format!("« {label} » : oui ou non.")),
            },
        },
        "lien" => super::super::shopping::safe_url(v).map(|u| json!(u)).ok_or_else(|| format!("« {label} » doit être une adresse web.")),
        "image" => {
            let u = super::super::shopping::safe_image(v);
            if u.is_empty() { Err(format!("« {label} » doit être une adresse https d'image.")) } else { Ok(json!(u)) }
        }
        _ => Ok(json!(clean_text(&js::s(v), 200))),
    }
}

/// Valeur pour l'affichage : « 18 € », « 12/03/2024 », « Oui ».
pub fn format_value(field: &J, v: &J) -> String {
    // Les fiches déjà enregistrées avec un « null » écrit en toutes lettres s'affichent vides.
    if v.is_null() || (v.is_string() && is_blank_text(&js::s(v))) {
        return String::new();
    }
    match js::gs(field, "type").as_str() {
        "prix" => js::num(v).map(super::super::shopping::fmt_price).unwrap_or_else(|| js::s(v)),
        "date" => date_fr(&js::s(v)),
        "oui_non" => if v.as_bool().unwrap_or(false) { "Oui".into() } else { "Non".into() },
        "nombre" => js::num(v).map(js::num_str).unwrap_or_else(|| js::s(v)),
        _ => js::s(v),
    }
}

// --- Éléments ------------------------------------------------------------------------------------

pub fn value<'a>(item: &'a J, key: &str) -> &'a J {
    js::get(&item["valeurs"], key)
}

pub fn display(list: &J, item: &J, key: &str) -> String {
    match field(list, key) {
        Some(f) => format_value(f, value(item, key)),
        None => String::new(),
    }
}

pub fn title_of(list: &J, item: &J) -> String {
    let t = display(list, item, &js::gs(list, "titre"));
    if t.is_empty() { "(sans titre)".into() } else { t }
}

pub fn subtitle_of(list: &J, item: &J) -> String {
    js::str_list(&list["sousTitre"]).iter().map(|k| display(list, item, k)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

pub fn identifiers_of(list: &J, item: &J) -> Vec<String> {
    js::str_list(&list["identifiant"]).iter().map(|k| display(list, item, k)).filter(|s| !s.is_empty()).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Statut {
    Possede,
    Souhaite,
    Inconnu,
}

impl Statut {
    pub fn code(self) -> &'static str {
        match self {
            Statut::Possede => "possede",
            Statut::Souhaite => "souhaite",
            Statut::Inconnu => "",
        }
    }
}

pub fn statut_of(list: &J, item: &J) -> Statut {
    let Some(key) = list["etat"].as_str() else { return Statut::Inconnu };
    let v = fold(&js::s(value(item, key)));
    let voc = vocab(list);
    if v.is_empty() {
        return Statut::Inconnu;
    }
    if v == fold(&js::gs(&voc, "fait")) || v.starts_with("possed") || v == "oui" || v.starts_with("achete") {
        Statut::Possede
    } else if v == fold(&js::gs(&voc, "voulu")) || v.starts_with("souhait") || v.starts_with("veux") || v.starts_with("voulu") || v.starts_with("envie") {
        Statut::Souhaite
    } else {
        Statut::Inconnu
    }
}

/// Le libellé du champ « statut » qui désigne « possédé » ou « souhaité » dans cette liste.
pub fn statut_label(list: &J, owned: bool) -> Option<String> {
    let key = list["etat"].as_str()?;
    let voc = vocab(list);
    let wanted = fold(&js::gs(&voc, if owned { "fait" } else { "voulu" }));
    let want = if owned { "possed" } else { "souhait" };
    let choices = js::str_list(&field(list, key)?["choix"]);
    choices.iter().find(|c| fold(c) == wanted).or_else(|| choices.iter().find(|c| fold(c).starts_with(want))).cloned()
}

thread_local! {
    /// (base de l'API d'images, jeton « img ») : donnés par l'application une fois la passerelle ouverte.
    static IMAGES: std::cell::RefCell<(String, String)> = const { std::cell::RefCell::new((String::new(), String::new())) };
}

pub fn set_image_base(base: &str, token: &str) {
    IMAGES.with(|i| *i.borrow_mut() = (base.to_string(), token.to_string()));
}

/// Image d'une fiche : copie locale (« local:jpg#1700000000 » → adresse de l'API d'images), sinon le lien du champ image.
pub fn image_of(list: &J, item: &J) -> String {
    let own = js::gs(item, "image");
    if let Some(rest) = own.strip_prefix("local:") {
        let (ext, stamp) = rest.split_once('#').unwrap_or((rest, "0"));
        let (base, token) = IMAGES.with(|i| i.borrow().clone());
        if base.is_empty() {
            return String::new();
        }
        return format!("{base}/{}/{}.{ext}?t={token}&v={stamp}", js::gs(list, "id"), js::gs(item, "id"));
    }
    if !own.is_empty() {
        return own;
    }
    list["image"].as_str().map(|k| js::s(value(item, k))).unwrap_or_default()
}

/// Élément tel que les fenêtres le reçoivent : valeurs brutes (formulaires) et valeurs mises en forme.
pub fn public_item(list: &J, item: &J) -> J {
    let mut shown = serde_json::Map::new();
    for f in js::arr(&list["champs"]) {
        let key = js::gs(&f, "cle");
        let text = format_value(&f, value(item, &key));
        if !text.is_empty() {
            shown.insert(key, json!(text));
        }
    }
    let badge = js::str_list(&list["vignette"]).iter().map(|k| display(list, item, k)).find(|s| !s.is_empty()).unwrap_or_default();
    json!({
        "id": item["id"], "titre": title_of(list, item), "sousTitre": subtitle_of(list, item),
        "statut": statut_of(list, item).code(), "vignette": badge, "image": image_of(list, item),
        "valeurs": item["valeurs"], "affichage": shown, "brouillon": js::gb(item, "brouillon"),
        "creeLe": item["creeLe"], "majLe": item["majLe"], "historiquePrix": item["historiquePrix"], "confiance": item["confiance"],
        "origine": item["origine"], "candidats": js::arr(&item["candidats"]).len(), "candidatIndex": item["candidatIndex"]
    })
}

pub fn counts(list: &J) -> (usize, usize, usize) {
    let items = js::arr(&list["elements"]);
    let owned = items.iter().filter(|i| statut_of(list, i) == Statut::Possede).count();
    let wish = items.iter().filter(|i| statut_of(list, i) == Statut::Souhaite).count();
    (items.len(), owned, wish)
}

/// Liste telle que les fenêtres la reçoivent (schéma + éléments mis en forme).
pub fn public_list(list: &J, with_items: bool) -> J {
    let (total, owned, wish) = counts(list);
    let mut out = json!({
        "id": list["id"], "nom": list["nom"], "icone": list["icone"], "noms": list["noms"], "alias": list["alias"],
        "modele": list["modele"], "champs": list["champs"], "titre": list["titre"], "sousTitre": list["sousTitre"],
        "identifiant": list["identifiant"], "etat": list["etat"], "vignette": list["vignette"], "image": list["image"],
        "actions": list["actions"], "enrichissement": list["enrichissement"], "creeLe": list["creeLe"], "majLe": list["majLe"],
        "origine": list["origine"], "vocabulaire": vocab(list), "tri": list["tri"],
        "total": total, "possedes": owned, "souhaites": wish
    });
    if with_items {
        out["elements"] = json!(js::arr(&list["elements"]).iter().map(|i| public_item(list, i)).collect::<Vec<_>>());
    }
    out
}

/// Texte indexé dans le RAG : tout ce que l'utilisateur pourrait demander sur cet élément.
pub fn passage(list: &J, item: &J) -> String {
    let singular = js::capitalize(&js::gs(&list["noms"], "singulier"));
    let mut text = format!("{singular} « {} » (liste {})", title_of(list, item), js::gs(list, "nom"));
    let sub = subtitle_of(list, item);
    if !sub.is_empty() {
        text.push_str(&format!(" — {sub}"));
    }
    text.push('.');
    match statut_of(list, item) {
        Statut::Possede => text.push_str(" Possédé."),
        Statut::Souhaite => text.push_str(" Souhaité, pas encore possédé."),
        Statut::Inconnu => {}
    }
    let skip: Vec<String> = [js::gs(list, "titre")].into_iter().chain(js::str_list(&list["sousTitre"])).chain(list["etat"].as_str().map(String::from)).collect();
    for f in js::arr(&list["champs"]) {
        let key = js::gs(&f, "cle");
        if skip.contains(&key) || ["image", "lien"].contains(&js::gs(&f, "type").as_str()) {
            continue;
        }
        let text_value = format_value(&f, value(item, &key));
        if !text_value.is_empty() {
            text.push_str(&format!(" {} : {}.", js::gs(&f, "libelle"), text_value.replace('\n', " ")));
        }
    }
    text
}

/// Résumé d'une liste pour la mémoire générale de Jarvis (jamais le détail des éléments).
pub fn summary(list: &J) -> String {
    let (total, owned, wish) = counts(list);
    let plural = js::gs(&list["noms"], "pluriel");
    let mut s = format!("Liste « {} » (aussi appelée : {}) : {total} {plural}", js::gs(list, "nom"), js::str_list(&list["alias"]).join(", "));
    if owned + wish > 0 {
        let voc = vocab(list);
        let (f1, fn_, v1, vn) = (js::gs(&voc, "fait"), js::gs(&voc, "faitPluriel"), js::gs(&voc, "voulu"), js::gs(&voc, "vouluPluriel"));
        s.push_str(&format!(" dont {owned} {} et {wish} {}", (if owned > 1 { fn_ } else { f1 }).to_lowercase(), (if wish > 1 { vn } else { v1 }).to_lowercase()));
    }
    s.push_str(&format!(". Champs : {}.", js::arr(&list["champs"]).iter().map(|f| js::gs(f, "libelle")).collect::<Vec<_>>().join(", ")));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orch::collections::Collections;
    use serde_json::Map;

    #[test]
    fn null_ecrit_en_toutes_lettres_est_vide() {
        let f = json!({ "cle": "marque", "libelle": "Marque", "type": "texte" });
        assert!(normalize_value(&f, &json!("null")).unwrap().is_null());
        assert!(normalize_value(&f, &json!(" N/A ")).unwrap().is_null());
        assert_eq!(format_value(&f, &json!("null")), "");
        assert_eq!(normalize_value(&f, &json!("Marshal")).unwrap(), json!("Marshal"));
        let list = json!({ "champs": [{ "cle": "nom", "type": "texte" }, { "cle": "marque", "type": "texte" }, { "cle": "notes", "type": "texte" }] });
        let mut vals = Map::new();
        vals.insert("nom".into(), json!("Enceinte"));
        assert!(Collections::has_gaps(&list, &vals), "la marque manque");
        vals.insert("marque".into(), json!("Marshal"));
        assert!(!Collections::has_gaps(&list, &vals), "les notes sont personnelles : jamais cherchées");
    }

    #[test]
    fn modele_demande_aligne_sur_le_site() {
        let mut l = json!({ "nom": "Noël", "champs": [{ "cle": "nom", "libelle": "Nom", "type": "texte" }, { "cle": "commentaire", "libelle": "Commentaire", "type": "texte_long" },
            { "cle": "etat", "libelle": "État", "type": "choix", "choix": ["Idée", "Commandé", "Offert"] }], "titre": "nom", "etat": "etat" });
        assert!(align_with_site(&mut l));
        assert_eq!(l["image"], "image");
        assert_eq!(l["enrichissement"]["fournisseur"], "web");
        let n = normalize_schema(&l).expect("modèle valide");
        assert_eq!(field(&n, "etat").unwrap()["choix"], json!(["Souhaité", "Possédé"]));
        assert!(field(&n, "image").is_some());
        let mut again = n.clone();
        assert!(!align_with_site(&mut again), "déjà aligné");
    }

    fn vinyl() -> J {
        let mut schema = normalize_schema(&preset("vinyle").unwrap()).unwrap();
        schema["id"] = json!("vinyle_test");
        schema["elements"] = json!([]);
        schema
    }

    #[test]
    fn les_modeles_livres_sont_valides() {
        for p in presets() {
            let s = normalize_schema(p).unwrap_or_else(|e| panic!("modèle {} invalide : {e}", js::gs(p, "id")));
            assert_eq!(js::arr(&s["champs"]).len(), js::arr(&p["champs"]).len(), "un champ du modèle {} a été rejeté", js::gs(p, "id"));
            assert_eq!(js::arr(&s["actions"]).len(), js::arr(&p["actions"]).len(), "une action du modèle {} a été rejetée", js::gs(p, "id"));
            assert!(s["etat"].is_string(), "le modèle {} n'a pas de statut", js::gs(p, "id"));
        }
    }

    #[test]
    fn detection_du_modele() {
        assert_eq!(js::gs(&detect_preset("une liste de disques vinyle").unwrap(), "id"), "vinyle");
        assert_eq!(js::gs(&detect_preset("liste de lego").unwrap(), "id"), "lego");
        assert!(detect_preset("liste de plantes").is_none());
    }

    #[test]
    fn le_schema_refuse_ce_qui_est_dangereux() {
        let s = normalize_schema(&json!({
            "nom": "Test", "champs": [
                { "libelle": "Nom", "type": "texte" }, { "libelle": "Nom", "type": "inconnu" },
                { "libelle": "Genre", "type": "choix", "choix": [] }
            ],
            "actions": [
                { "libelle": "Pirate", "type": "url", "gabarit": "http://exemple.fr/?q={nom}" },
                { "libelle": "Autre champ", "type": "url", "gabarit": "https://exemple.fr/?q={secret}" },
                { "libelle": "Accolades", "type": "url", "gabarit": "https://exemple.fr/?q={nom}{" },
                { "libelle": "Script", "type": "javascript", "gabarit": "alert(1)" },
                { "libelle": "Bon", "type": "url", "gabarit": "https://exemple.fr/?q={nom}" }
            ]
        }))
        .unwrap();
        let keys: Vec<String> = js::arr(&s["champs"]).iter().map(|f| js::gs(f, "cle")).collect();
        assert_eq!(keys, vec!["nom", "nom_2", "genre"]);
        assert_eq!(js::gs(&s["champs"][1], "type"), "texte");
        assert_eq!(js::gs(&s["champs"][2], "type"), "texte", "un choix sans options devient un texte");
        let actions = js::arr(&s["actions"]);
        assert_eq!(actions.len(), 1);
        assert_eq!(js::gs(&actions[0], "libelle"), "Bon");
        assert!(normalize_schema(&json!({ "nom": "", "champs": [{ "libelle": "x" }] })).is_err());
        assert!(normalize_schema(&json!({ "nom": "Vide", "champs": [] })).is_err());
    }

    #[test]
    fn les_champs_sont_plafonnes() {
        let many: Vec<J> = (0..30).map(|i| json!({ "libelle": format!("Champ {i}") })).collect();
        let s = normalize_schema(&json!({ "nom": "Gros", "champs": many })).unwrap();
        assert_eq!(js::arr(&s["champs"]).len(), MAX_FIELDS);
    }

    #[test]
    fn validation_des_valeurs() {
        let l = vinyl();
        let f = |k: &str| field(&l, k).unwrap().clone();
        assert_eq!(normalize_value(&f("prix_achat"), &json!("25,90 €")).unwrap(), json!(25.9));
        assert_eq!(normalize_value(&f("prix_achat"), &json!("18 euros")).unwrap(), json!(18.0));
        assert!(normalize_value(&f("prix_achat"), &json!("beaucoup")).is_err());
        assert_eq!(normalize_value(&f("annee"), &json!("1 983")).unwrap(), json!(1983));
        assert_eq!(normalize_value(&f("date_achat"), &json!("12/03/2024")).unwrap(), json!("2024-03-12"));
        assert_eq!(normalize_value(&f("date_achat"), &json!("5.1.24")).unwrap(), json!("2024-01-05"));
        assert!(normalize_value(&f("date_achat"), &json!("31/02/2024")).is_err());
        assert_eq!(normalize_value(&f("etat"), &json!("possede")).unwrap(), json!("Possédé"));
        assert_eq!(normalize_value(&f("etat"), &json!("souhaité")).unwrap(), json!("Souhaité"));
        assert!(normalize_value(&f("etat"), &json!("peut-être")).is_err());
        assert_eq!(normalize_value(&f("titre"), &json!("  Under   a\nBlood Red Sky ")).unwrap(), json!("Under a Blood Red Sky"));
        assert_eq!(normalize_value(&f("titre"), &json!("  ")).unwrap(), Value::Null);
        assert!(normalize_value(&f("pochette"), &json!("http://exemple.fr/a.jpg")).is_err(), "une image doit être en https");
        assert!(normalize_value(&f("pochette"), &json!("javascript:alert(1)")).is_err());
        assert!(normalize_value(&f("pochette"), &json!("https://exemple.fr/a.jpg")).is_ok());
    }

    #[test]
    fn vocabulaire_vu_a_voir() {
        let film = normalize_schema(&preset("film").unwrap()).unwrap();
        let it = |s: &str| json!({ "id": "e1", "valeurs": { "titre": "Inception", "etat": s } });
        assert_eq!(statut_of(&film, &it("Vu")), Statut::Possede);
        assert_eq!(statut_of(&film, &it("À voir")), Statut::Souhaite);
        assert_eq!(statut_label(&film, false).as_deref(), Some("À voir"));
        assert_eq!(statut_label(&film, true).as_deref(), Some("Vu"));
        assert_eq!(js::gs(&vocab(&film), "faitPluriel"), "Vus");
        assert!(!default_is_done(&film, &serde_json::Map::new()), "un film ajouté sans précision est à voir");
        let serie = normalize_schema(&preset("serie").unwrap()).unwrap();
        assert_eq!(statut_label(&serie, true).as_deref(), Some("Vue"));
        // Concert : passé = vu, à venir = à voir.
        let concert = normalize_schema(&preset("concert").unwrap()).unwrap();
        let mut past = serde_json::Map::new();
        past.insert("date".into(), json!("2020-01-01"));
        let mut future = serde_json::Map::new();
        future.insert("date".into(), json!("2999-01-01"));
        assert!(default_is_done(&concert, &past));
        assert!(!default_is_done(&concert, &future));
        // Les listes d'avant le vocabulaire gardent « possédé / souhaité ».
        let vinyl = normalize_schema(&preset("vinyle").unwrap()).unwrap();
        assert_eq!(js::gs(&vocab(&vinyl), "fait"), "Possédé");
        let mut old = vinyl.clone();
        old.as_object_mut().unwrap().remove("vocabulaire");
        assert_eq!(js::gs(&vocab(&old), "voulu"), "Souhaité");
        assert!(summary(&film).contains("Films"));
    }

    #[test]
    fn lecture_d_un_element() {
        let l = vinyl();
        let item = json!({ "id": "e1", "valeurs": {
            "titre": "Under a Blood Red Sky", "artiste": "U2", "annee": 1983, "etat": "Possédé",
            "prix_achat": 18.0, "date_achat": "2024-03-12", "lieu_achat": "Fnac Strasbourg" } });
        assert_eq!(title_of(&l, &item), "Under a Blood Red Sky");
        assert_eq!(subtitle_of(&l, &item), "U2 · 1983");
        assert_eq!(statut_of(&l, &item), Statut::Possede);
        let p = passage(&l, &item);
        assert!(p.contains("U2") && p.contains("Possédé") && p.contains("18 €") && p.contains("12/03/2024") && p.contains("Fnac Strasbourg"), "{p}");
        let public = public_item(&l, &item);
        assert_eq!(public["vignette"], json!("18 €"));
        assert_eq!(public["statut"], json!("possede"));
        assert_eq!(statut_label(&l, false).as_deref(), Some("Souhaité"));
    }

    #[test]
    fn un_set_lego_se_retrouve_par_son_numero() {
        let mut l = normalize_schema(&preset("lego").unwrap()).unwrap();
        l["id"] = json!("lego_test");
        let item = json!({ "id": "e1", "valeurs": { "numero": "75436", "nom": "Le Faucon Millenium", "etat": "Souhaité" } });
        assert_eq!(identifiers_of(&l, &item), vec!["75436"]);
        assert_eq!(statut_of(&l, &item), Statut::Souhaite);
        assert_eq!(title_of(&l, &item), "Le Faucon Millenium");
    }
}
