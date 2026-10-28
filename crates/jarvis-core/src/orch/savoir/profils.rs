// Profils des dossiers Data : chaque dossier a ses spécificités. Un produit (aspirateur, hélicoptère…) collecte prix,
// points forts et faibles, avis, pièces et finit par l'explorateur 3D ; de la musique collecte de quoi l'écouter
// (Spotify, YouTube) et finit par l'écran médias ; un film, un lieu, un sujet général ont les leurs. Les profils
// livrés sont dans data/savoir-profils.json ; pour un sujet qui n'entre dans aucun, Haiku en crée un, relu ici champ
// par champ et gardé dans data/savoir/profils.json pour les dossiers suivants du même genre.
// Le profil est choisi à la création du dossier (mots-clés, sinon Haiku) et noté dans meta.profil.

use serde_json::{json, Value};

use super::dossier::{ident, url_ok};
use super::Savoir;
use crate::js::{self, J};
use crate::orch::Up;

const BUILTIN: &str = include_str!("../../../data/savoir-profils.json");

/// Champs facultatifs qu'un profil peut demander à la recherche (relus par dossier::validate).
pub const CHAMPS: &[&str] = &["points_forts", "points_faibles", "prix", "avis", "ecoute", "pieces", "modeles3d", "maquette"];
/// Fin de la présentation : explorateur 3D, écran médias (écoute, vidéos), ou liens.
pub const FINALES: &[&str] = &["3d", "media", "liens"];
const MAX_PERSO: usize = 40;

const CLASS_PROMPT: &str = "Tu ranges un sujet de recherche dans un profil de dossier. Réponds uniquement par un objet JSON, sans texte autour. Si l'un des profils fournis convient, réponds {\"profil\":\"<id exact>\"}. Sinon, crée un profil réutilisable pour ce genre de sujet (pas pour ce sujet seul) : {\"profil\":\"nouveau\",\"nouveau\":{\"nom\":\"<genre, 30 caractères au plus>\",\"icone\":\"<un symbole>\",\"description\":\"<les sujets qu'il couvre>\",\"mots\":[\"<mots-clés en minuscules sans accents>\"],\"champs\":[<parmi \"points_forts\",\"points_faibles\",\"prix\",\"avis\",\"ecoute\",\"pieces\",\"modeles3d\",\"maquette\">],\"consigne\":\"<ce que la recherche doit relever pour ce genre de sujet, 600 caractères au plus>\",\"troisD\":<true seulement pour un objet physique qu'on peut montrer en 3D>,\"finale\":\"<3d, media (écoute ou vidéos) ou liens>\",\"scenes\":[<types parmi titre, image, chiffres, fiche, chronologie, comparaison, piece, video, atouts, prix, avis, ecoute, liens, conclusion>],\"boutons\":[{\"libelle\":\"<lien utile>\",\"icone\":\"<un symbole>\",\"type\":\"url\",\"gabarit\":\"https://…{sujet}…\"}]}}. Les boutons (2 à 4) sont des recherches sur des sites de référence du domaine, avec {sujet} à la place du sujet.";

/// Profils livrés avec Jarvis.
pub fn builtin() -> Vec<J> {
    serde_json::from_str::<J>(BUILTIN).map(|v| js::arr(&v["profils"])).unwrap_or_default()
}

/// Profil proposé (par Haiku, ou relu d'un fichier) → profil sûr, ou None.
pub fn validate(raw: &J) -> Option<J> {
    let nom = js::slice_chars(js::gs(raw, "nom").trim(), 40);
    let id = Some(js::gs(raw, "id")).filter(|s| !s.is_empty()).map(|s| ident(&s, 30)).unwrap_or_else(|| ident(&nom, 30));
    if nom.is_empty() || id.is_empty() {
        return None;
    }
    let scene_types = super::present::TYPES;
    let mut scenes: Vec<String> = js::str_list(&raw["scenes"]).into_iter().filter(|s| scene_types.contains(&s.as_str()) && !["exploration", "media"].contains(&s.as_str())).collect();
    scenes.dedup();
    if scenes.first().map(String::as_str) != Some("titre") {
        scenes.retain(|s| s != "titre");
        scenes.insert(0, "titre".into());
    }
    if scenes.last().map(String::as_str) != Some("conclusion") {
        scenes.retain(|s| s != "conclusion");
        scenes.push("conclusion".into());
    }
    let troisd = js::gb(raw, "troisD");
    let finale = Some(js::gs(raw, "finale")).filter(|f| FINALES.contains(&f.as_str()) && (f != "3d" || troisd)).unwrap_or_else(|| "liens".into());
    let mut champs: Vec<String> = js::str_list(&raw["champs"]).into_iter().filter(|c| CHAMPS.contains(&c.as_str())).collect();
    champs.dedup();
    if !troisd {
        champs.retain(|c| !["pieces", "modeles3d", "maquette"].contains(&c.as_str()));
    }
    let boutons: Vec<J> = js::arr(&raw["boutons"])
        .iter()
        .filter_map(|b| {
            let (libelle, gabarit) = (js::slice_chars(js::gs(b, "libelle").trim(), 40), js::slice_chars(js::gs(b, "gabarit").trim(), 300));
            let kind = js::gs(b, "type");
            let ok = !libelle.is_empty() && gabarit.contains("{sujet}") && match kind.as_str() {
                "url" => url_ok(&gabarit.replace("{sujet}", "x"), false) && gabarit.starts_with("https://"),
                "spotify" => !gabarit.contains("://"),
                _ => false,
            };
            ok.then(|| json!({ "libelle": libelle, "icone": js::slice_chars(js::gs(b, "icone").trim(), 3), "type": kind, "gabarit": gabarit }))
        })
        .take(5)
        .collect();
    let mut mots: Vec<String> = js::str_list(&raw["mots"]).into_iter().map(|m| ident(&m, 30)).filter(|m| !m.is_empty() && !m.contains('-')).collect();
    mots.dedup();
    mots.truncate(30);
    Some(json!({
        "id": id, "nom": nom, "icone": Some(js::slice_chars(js::gs(raw, "icone").trim(), 3)).filter(|i| !i.is_empty()).unwrap_or_else(|| "◈".into()),
        "description": js::slice_chars(js::gs(raw, "description").trim(), 300), "mots": mots, "champs": champs,
        "consigne": js::slice_chars(js::gs(raw, "consigne").trim(), 900), "troisD": troisd, "finale": finale, "scenes": scenes, "boutons": boutons
    }))
}

/// Profil reconnu aux mots du sujet (« aspirateur Dyson V15 » → produit), sans IA. Le plus de mots communs gagne.
pub fn quick_match(profils: &[J], sujet: &str) -> Option<String> {
    let said: Vec<String> = js::fold_lower(sujet).split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| w.len() >= 2).map(String::from).collect();
    profils
        .iter()
        .map(|p| (js::str_list(&p["mots"]).iter().filter(|m| said.contains(m)).count(), js::gs(p, "id")))
        .filter(|(n, _)| *n > 0)
        .max_by_key(|(n, _)| *n)
        .map(|(_, id)| id)
}

/// Liens utiles du profil pour un sujet : { libelle, icone, type, url | requete }.
pub fn buttons(profil: &J, sujet: &str) -> Vec<J> {
    js::arr(&profil["boutons"])
        .iter()
        .map(|b| {
            let g = js::gs(b, "gabarit");
            let mut out = json!({ "libelle": b["libelle"], "icone": b["icone"], "type": b["type"] });
            if js::gs(b, "type") == "spotify" {
                out["requete"] = json!(g.replace("{sujet}", sujet));
            } else {
                out["url"] = json!(g.replace("{sujet}", &urlencoding::encode(sujet)));
            }
            out
        })
        .collect()
}

impl Savoir {
    fn profils_file(&self) -> std::path::PathBuf {
        self.dir.join("profils.json")
    }

    /// Profils créés par l'IA.
    fn custom_profiles(&self) -> Vec<J> {
        js::read_json(&self.profils_file()).map(|v| js::arr(&v).iter().filter_map(validate).collect()).unwrap_or_default()
    }

    /// Tous les profils : livrés, puis créés par l'IA.
    pub fn profiles(&self) -> Vec<J> {
        let mut all = builtin();
        let known: Vec<String> = all.iter().map(|p| js::gs(p, "id")).collect();
        all.extend(self.custom_profiles().into_iter().filter(|p| !known.contains(&js::gs(p, "id"))));
        all
    }

    /// Profil d'identifiant `id` (le sujet général à défaut).
    pub fn profile(&self, id: &str) -> J {
        let all = self.profiles();
        all.iter().find(|p| js::gs(p, "id") == id).or_else(|| all.iter().find(|p| js::gs(p, "id") == "general")).cloned().unwrap_or(json!({ "id": "general", "nom": "Sujet général", "finale": "liens" }))
    }

    /// Profil d'un dossier (noté à sa création).
    pub fn profile_of(&self, slug: &str) -> J {
        self.profile(&self.get(slug).map(|d| js::gs(&d["meta"], "profil")).unwrap_or_default())
    }

    /// Profil d'un nouveau sujet : mots-clés, sinon Haiku, qui choisit un profil ou en crée un (gardé pour la suite).
    pub async fn choose_profile(&self, sujet: &str) -> String {
        let all = self.profiles();
        if let Some(id) = quick_match(&all, sujet) {
            return id;
        }
        let input = serde_json::to_string(&json!({
            "sujet": sujet,
            "profils": all.iter().map(|p| json!({ "id": p["id"], "nom": p["nom"], "description": p["description"] })).collect::<Vec<_>>()
        }))
        .unwrap_or_default();
        let o = self.o.up();
        let Some(raw) = super::present::haiku(&o, CLASS_PROMPT, &input, &format!("Profil · {sujet}")).await.and_then(|t| super::dossier::parse_loose(&t)) else {
            return "general".into();
        };
        let chosen = js::gs(&raw, "profil");
        if all.iter().any(|p| js::gs(p, "id") == chosen) {
            return chosen;
        }
        let Some(p) = validate(&raw["nouveau"]) else { return "general".into() };
        let id = js::gs(&p, "id");
        if all.iter().any(|x| js::gs(x, "id") == id) {
            return id; // même genre déjà connu
        }
        let mut custom: Vec<J> = js::read_json(&self.profils_file()).map(|v| js::arr(&v)).unwrap_or_default();
        custom.push(p.clone());
        let skip = custom.len().saturating_sub(MAX_PERSO);
        let _ = js::write_json_atomic(&self.profils_file(), &Value::Array(custom.into_iter().skip(skip).collect()));
        (self.log)(format!("Data : nouveau profil de dossier « {} » créé pour {sujet}", js::gs(&p, "nom")));
        o.record("savoir_profil_cree", json!({ "profil": id, "sujet": sujet }));
        id
    }

    /// Profil du dossier, choisi au besoin (dossiers créés avant les profils) puis noté.
    pub async fn ensure_profile(&self, slug: &str) -> J {
        let current = self.get(slug).map(|d| js::gs(&d["meta"], "profil")).unwrap_or_default();
        if !current.is_empty() && self.profiles().iter().any(|p| js::gs(p, "id") == current) {
            return self.profile(&current);
        }
        let sujet = self.get(slug).map(|d| js::gs(&d, "sujet")).unwrap_or_default();
        let id = self.choose_profile(&sujet).await;
        if let Some(d) = self.dossiers.borrow_mut().get_mut(slug) {
            d["meta"]["profil"] = json!(id);
        }
        self.save(slug);
        self.profile(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profils_livres_valides() {
        let all = builtin();
        let ids: Vec<String> = all.iter().map(|p| js::gs(p, "id")).collect();
        assert_eq!(ids, ["general", "produit", "musique", "film", "lieu"]);
        for p in &all {
            let v = validate(p).unwrap_or_else(|| panic!("profil {} invalide", js::gs(p, "id")));
            assert_eq!(v["boutons"].as_array().map(|a| a.len()), p["boutons"].as_array().map(|a| a.len()), "lien refusé dans {}", js::gs(p, "id"));
            assert_eq!(v["champs"], p["champs"], "champ inconnu dans {}", js::gs(p, "id"));
            assert_eq!(v["scenes"], p["scenes"], "scène inconnue dans {}", js::gs(p, "id"));
        }
    }

    #[test]
    fn profil_reconnu_aux_mots_du_sujet() {
        let all = builtin();
        assert_eq!(quick_match(&all, "l'aspirateur Dyson V15 Detect").as_deref(), Some("produit"));
        assert_eq!(quick_match(&all, "Hélicoptère H160").as_deref(), Some("produit"));
        assert_eq!(quick_match(&all, "le concert de Daft Punk à Bercy").as_deref(), Some("musique"));
        assert_eq!(quick_match(&all, "la série The Office").as_deref(), Some("film"));
        assert_eq!(quick_match(&all, "le château de Chambord").as_deref(), Some("lieu"));
        assert!(quick_match(&all, "les orchidées").is_none(), "inconnu : Haiku décide");
    }

    #[test]
    fn profil_cree_par_l_ia_relu() {
        let raw = json!({ "nom": "Plante", "icone": "🌿", "mots": ["Orchidée", "plante verte"], "champs": ["points_forts", "pieces", "inventé"],
            "troisD": false, "finale": "3d", "scenes": ["image", "inventee", "liens"],
            "boutons": [{ "libelle": "Entretien", "type": "url", "gabarit": "https://www.google.com/search?q={sujet}+entretien" },
                        { "libelle": "Local", "type": "url", "gabarit": "http://192.168.1.1/{sujet}" },
                        { "libelle": "Sans sujet", "type": "url", "gabarit": "https://example.org/" }] });
        let p = validate(&raw).unwrap();
        assert_eq!(p["id"], "plante");
        assert_eq!(p["champs"], json!(["points_forts"]), "pas de champ inconnu ni de 3D sans troisD");
        assert_eq!(p["finale"], "liens", "pas d'explorateur 3D sans troisD");
        assert_eq!(p["scenes"], json!(["titre", "image", "liens", "conclusion"]));
        assert_eq!(p["boutons"].as_array().unwrap().len(), 1, "adresse locale et lien sans {{sujet}} refusés");
        assert_eq!(p["mots"], json!(["orchidee"]));
        let b = buttons(&builtin()[2], "Daft Punk");
        assert_eq!(b[0]["requete"], "Daft Punk");
        assert_eq!(b[1]["url"], "https://www.youtube.com/results?search_query=Daft%20Punk");
    }
}
