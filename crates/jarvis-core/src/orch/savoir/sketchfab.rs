// Modèles 3D réalistes d'un dossier Data, trouvés sur Sketchfab (API publique, sans compte) et affichés par la
// visionneuse officielle dans l'explorateur 3D (ui/viewer3d.js). Une maquette de formes simples ne peut pas
// ressembler au produit : un modèle fait par un graphiste, si.
//   - recherche : sujet du dossier, classement par pertinence du nom, animation, finesse et popularité ; les
//     modèles trop simples ou qui ne portent pas la référence du produit (« H160 ») sont écartés ;
//   - pièces : les nœuds nommés du modèle (Rotor_ROT, LandingGears…) sont reliés aux pièces du dossier par
//     Haiku, une fois par modèle, pour les sélectionner, les écarter (vue éclatée) ou masquer l'enveloppe
//     (rayons X).
// Tout est gardé dans data/savoir/<slug>/sketchfab.json (cache de 30 jours pour la recherche).

use std::collections::HashSet;

use serde_json::{json, Value};

use super::Savoir;
use crate::js::{self, J};
use crate::orch::Up;

const CACHE_MS: i64 = 30 * 24 * 3600 * 1000;
const MAX_MODELES: usize = 6;
const MIN_FACES: u64 = 2000;
const MAX_NOEUDS: usize = 400;

const MAP_PROMPT: &str = "Tu relies les pièces d'un produit, décrites en français, aux nœuds nommés d'un modèle 3D (noms souvent en anglais, écrits par le graphiste). Réponds uniquement par un objet JSON {\"pieces\":{\"<id exact de la pièce>\":[\"<nom exact de nœud>\", …]},\"enveloppe\":[\"<nom exact de nœud>\", …]}, sans texte autour. Pour chaque pièce, donne le ou les nœuds qui la représentent le mieux (de préférence un nœud de groupe, comme Rotor_ROT plutôt que chaque pale) ; omets une pièce sans nœud correspondant. \"enveloppe\" : les nœuds de la coque, du fuselage, de la carrosserie ou du capot qu'il faut masquer pour voir l'intérieur (vitres comprises). N'utilise que des noms présents dans la liste.";

/// Mots qui désignent le produit (« h160 » dans « Hélicoptère H160 ») : ceux qui portent un chiffre, sinon tous.
pub fn key_tokens(sujet: &str) -> Vec<String> {
    let all = super::words(sujet);
    let with_digits: Vec<String> = all.iter().filter(|w| w.chars().any(|c| c.is_ascii_digit())).cloned().collect();
    if with_digits.is_empty() { all } else { with_digits }
}

/// Texte réduit à ses lettres et chiffres (« H-160 » → « h160 »).
fn squash(s: &str) -> String {
    js::fold_lower(s).chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Résultats de recherche Sketchfab → modèles classés (les meilleurs d'abord), sans doublon.
pub fn rank(results: &[J], sujet: &str) -> Vec<J> {
    let keys = key_tokens(sujet);
    let others: Vec<String> = super::words(sujet).into_iter().filter(|w| !keys.contains(w)).collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut scored: Vec<(f64, J)> = vec![];
    for m in results {
        let uid = js::gs(m, "uid");
        let name = js::gs(m, "name");
        let faces = m["faceCount"].as_u64().unwrap_or(0);
        if uid.is_empty() || !seen.insert(uid.clone()) || js::gs(m, "embedUrl").is_empty() || js::gb(m, "isAgeRestricted") || faces < MIN_FACES {
            continue;
        }
        let tags: Vec<String> = js::arr(&m["tags"]).iter().map(|t| js::gs(t, "name")).collect();
        let flat = squash(&format!("{name} {}", tags.join(" ")));
        let name_flat = squash(&name);
        let hits = keys.iter().filter(|k| name_flat.contains(&squash(k))).count();
        if hits == 0 && !keys.iter().any(|k| flat.contains(&squash(k))) {
            continue; // ne porte pas la référence du produit
        }
        let anim = m["animationCount"].as_u64().unwrap_or(0);
        let likes = m["likeCount"].as_u64().unwrap_or(0) as f64;
        let mut score = 10.0 * hits as f64 + 2.0 * others.iter().filter(|w| name_flat.contains(&squash(w))).count() as f64;
        score += if anim > 0 { 3.0 } else { 0.0 };
        score += if faces >= 20_000 { 2.0 } else { 0.0 } + if faces >= 100_000 { 1.0 } else { 0.0 };
        score += (likes + 1.0).log10();
        score += if m["staffpickedAt"].is_null() { 0.0 } else { 2.0 };
        let thumb = js::arr(&m["thumbnails"]["images"])
            .into_iter()
            .min_by_key(|i| (i["width"].as_i64().unwrap_or(0) - 320).abs())
            .map(|i| js::gs(&i, "url"))
            .unwrap_or_default();
        let licence = js::gs(&m["license"], "label");
        scored.push((score, json!({
            "uid": uid, "nom": name, "auteur": m["user"]["displayName"].as_str().or(m["user"]["username"].as_str()).unwrap_or(""),
            "licence": if licence.is_empty() { "visionneuse Sketchfab".to_string() } else { licence },
            "anime": anim > 0, "faces": faces, "vignette": thumb, "page": m["viewerUrl"]
        })));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(MAX_MODELES).map(|(_, m)| m).collect()
}

/// Correspondance proposée par l'IA → seulement des pièces du dossier et des nœuds du modèle.
pub fn clean_mapping(raw: &J, piece_ids: &[String], nodes: &[String]) -> J {
    let known: HashSet<&str> = nodes.iter().map(String::as_str).collect();
    let keep = |v: &J| -> Vec<String> {
        let mut out: Vec<String> = js::str_list(v).into_iter().filter(|n| known.contains(n.as_str())).collect();
        out.dedup();
        out.truncate(12);
        out
    };
    let mut pieces = serde_json::Map::new();
    if let Some(m) = raw["pieces"].as_object() {
        for (id, v) in m {
            let names = keep(v);
            if piece_ids.contains(id) && !names.is_empty() {
                pieces.insert(id.clone(), json!(names));
            }
        }
    }
    let mut env = keep(&raw["enveloppe"]);
    env.truncate(30);
    json!({ "pieces": Value::Object(pieces), "enveloppe": env })
}

async fn search_api(q: &str) -> Result<Vec<J>, String> {
    let url = format!("https://api.sketchfab.com/v3/search?type=models&count=24&q={}", urlencoding::encode(q));
    let resp = crate::net::http().get(&url).timeout(std::time::Duration::from_secs(20)).send().await.map_err(|e| format!("réseau : {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Sketchfab a répondu {}", resp.status().as_u16()));
    }
    let v: J = resp.json().await.map_err(|e| e.to_string())?;
    Ok(js::arr(&v["results"]))
}

impl Savoir {
    fn sketchfab_file(&self, slug: &str) -> std::path::PathBuf {
        self.dir.join(slug).join("sketchfab.json")
    }

    fn sketchfab_cache(&self, slug: &str) -> J {
        js::read_json(&self.sketchfab_file(slug)).unwrap_or(json!({}))
    }

    fn sketchfab_save(&self, slug: &str, v: &J) {
        let _ = js::write_json_atomic(&self.sketchfab_file(slug), v);
    }

    /// Modèles réalistes déjà trouvés (cache), sans recherche.
    pub fn sketchfab_models(&self, slug: &str) -> Vec<J> {
        js::arr(&self.sketchfab_cache(slug)["modeles"])
    }

    /// Modèles réalistes du dossier : le cache s'il a moins de 30 jours, sinon une recherche (sujet entier, puis sa
    /// référence seule). Liste vide hors ligne ou sans résultat pertinent.
    pub async fn find_models(&self, slug: &str) -> Vec<J> {
        let cache = self.sketchfab_cache(slug);
        if js::now_ms() - cache["at"].as_i64().unwrap_or(0) < CACHE_MS && cache["modeles"].is_array() {
            return js::arr(&cache["modeles"]);
        }
        let Some(d) = self.get(slug) else { return vec![] };
        let sujet = js::gs(&d, "sujet");
        let mut results: Vec<J> = vec![];
        let short = key_tokens(&sujet).join(" ");
        for q in [sujet.clone(), short].into_iter().collect::<Vec<_>>().iter().filter(|q| !q.trim().is_empty()) {
            match search_api(q).await {
                Ok(r) => results.extend(r),
                Err(e) => {
                    (self.log)(format!("Data : recherche de modèles 3D impossible ({e})"));
                    return js::arr(&cache["modeles"]);
                }
            }
            if rank(&results, &sujet).len() >= 3 {
                break;
            }
        }
        let models = rank(&results, &sujet);
        let mut cache = cache;
        cache["at"] = json!(js::now_ms());
        cache["modeles"] = json!(models);
        self.sketchfab_save(slug, &cache);
        self.o.up().record("savoir_modeles_3d", json!({ "dossier": slug, "trouves": models.len() }));
        models
    }

    /// Pièces du dossier ↔ nœuds du modèle `uid` (noms envoyés par la visionneuse). Calculé une fois par Haiku.
    pub async fn model_pieces(&self, slug: &str, uid: &str, nodes: &[String]) -> J {
        let mut cache = self.sketchfab_cache(slug);
        if cache["pieces"][uid].is_object() {
            return cache["pieces"][uid].clone();
        }
        let Some(d) = self.get(slug) else { return json!({ "pieces": {}, "enveloppe": [] }) };
        let pieces = js::arr(&d["pieces"]);
        let ids: Vec<String> = pieces.iter().map(|p| js::gs(p, "id")).collect();
        let mut names: Vec<String> = nodes.iter().map(|n| js::slice_chars(n, 80)).filter(|n| !n.is_empty()).collect();
        names.dedup();
        names.truncate(MAX_NOEUDS);
        if ids.is_empty() || names.is_empty() {
            return json!({ "pieces": {}, "enveloppe": [] });
        }
        let input = serde_json::to_string(&json!({
            "sujet": d["sujet"],
            "pieces": pieces.iter().map(|p| json!({ "id": p["id"], "nom": p["nom"], "description": js::slice_chars(&js::gs(p, "description"), 160) })).collect::<Vec<_>>(),
            "noeuds": names
        }))
        .unwrap_or_default();
        let o = self.o.up();
        let raw = super::present::haiku(&o, MAP_PROMPT, &input, &format!("Pièces 3D · {}", js::gs(&d, "sujet"))).await.and_then(|t| super::dossier::parse_loose(&t));
        let Some(raw) = raw else { return json!({ "pieces": {}, "enveloppe": [] }) }; // pas gardé : réessayé la prochaine fois
        let mapping = clean_mapping(&raw, &ids, &names);
        if !cache["pieces"].is_object() {
            cache["pieces"] = json!({});
        }
        cache["pieces"][uid] = mapping.clone();
        self.sketchfab_save(slug, &cache);
        mapping
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(uid: &str, name: &str, faces: u64, anim: u64, likes: u64) -> J {
        json!({ "uid": uid, "name": name, "faceCount": faces, "animationCount": anim, "likeCount": likes,
                "embedUrl": format!("https://sketchfab.com/models/{uid}/embed"), "viewerUrl": format!("https://sketchfab.com/3d-models/{uid}"),
                "user": { "displayName": "Auteur" }, "license": null,
                "thumbnails": { "images": [{ "width": 1024, "url": "https://a/1024.jpg" }, { "width": 256, "url": "https://a/256.jpg" }] } })
    }

    #[test]
    fn modeles_classes_et_filtres() {
        let r = vec![
            result("a", "Airbus H160 Helicopter", 32, 0, 50),
            result("b", "Airbus Helicopters H160", 1_004_566, 0, 46),
            result("c", "Helicopter H160", 623_011, 1, 9),
            result("d", "Aviation Headset David Clark H10-13S", 17_202, 0, 3),
            result("c", "Helicopter H160", 623_011, 1, 9),
        ];
        let m = rank(&r, "Hélicoptère H160");
        let uids: Vec<String> = m.iter().map(|x| js::gs(x, "uid")).collect();
        assert_eq!(uids, ["c", "b"], "trop simple, hors sujet et doublon écartés ; l'animé passe devant");
        assert_eq!(m[0]["vignette"], "https://a/256.jpg");
        assert_eq!(m[0]["licence"], "visionneuse Sketchfab");
        assert_eq!(key_tokens("Hélicoptère Airbus H160"), ["h160"]);
        assert_eq!(key_tokens("Tour Eiffel"), ["tour", "eiffel"]);
    }

    #[test]
    fn correspondance_bornee_au_modele() {
        let raw = json!({ "pieces": { "rotor-principal": ["Rotor_ROT", "Inventé"], "train": ["LandingGears"], "inconnue": ["mainHull"] }, "enveloppe": ["mainHull", "glass_mesh", "faux"] });
        let ids = vec!["rotor-principal".to_string(), "train".to_string()];
        let nodes: Vec<String> = ["Rotor_ROT", "LandingGears", "mainHull", "glass_mesh"].iter().map(|s| s.to_string()).collect();
        let m = clean_mapping(&raw, &ids, &nodes);
        assert_eq!(m["pieces"]["rotor-principal"], json!(["Rotor_ROT"]));
        assert!(m["pieces"]["inconnue"].is_null());
        assert_eq!(m["enveloppe"], json!(["mainHull", "glass_mesh"]));
    }
}
