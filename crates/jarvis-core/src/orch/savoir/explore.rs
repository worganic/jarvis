// Fin de présentation d'un dossier Data, selon son profil : explorateur 3D ou écran médias.
// Explorateur 3D d'un dossier Data (ui/viewer3d.js) : le produit en grand par-dessus le HUD, maquette de l'IA ou
// modèle .glb libre, pièces à cliquer, fonctionnement, vue éclatée, liens pour aller plus loin. Ouvert à la fin d'une
// présentation (scène « exploration »), par la card Data (bouton ◆ 3D) ou à la voix (« montre-moi le H160 en 3D »).
// Le HUD signale sa fermeture (op « explorer_ferme ») ; tant qu'il est ouvert, « fais-le fonctionner », « vue
// éclatée », « montre le rotor »… le pilotent et une question libre reçoit une réponse tirée du dossier.
// Écran médias (ui/dossier-media.js, profils musique, film…) : de quoi écouter (Spotify, par la lecture de Jarvis) et
// regarder (lecteur YouTube), points forts et faibles, prix, avis, liens du profil.

use std::collections::HashSet;

use base64::Engine;
use serde_json::json;

use super::Savoir;
use crate::js::{self, J};
use crate::orch::Up;

/// Plafond d'un modèle .glb envoyé au HUD (adresse data:).
const GLB_VIEW_MAX: u64 = 25 * 1024 * 1024;

/// Le dossier a-t-il de quoi être montré en 3D : modèle réaliste trouvé sur Sketchfab (clé « sketchfab », ajoutée
/// au dossier par l'appelant depuis le cache), modèle .glb téléchargé ou maquette de l'IA ?
pub fn has_3d(d: &J) -> bool {
    d["maquette"]["elements"].as_array().map(|a| !a.is_empty()).unwrap_or(false) || !models(d).is_empty() || d["sketchfab"].as_array().map(|a| !a.is_empty()).unwrap_or(false)
}

/// Identifiant d'une vidéo YouTube (watch?v=, youtu.be/, embed/, shorts/, live/), pour le lecteur intégré.
pub fn youtube_id(url: &str) -> Option<String> {
    crate::re!(r"^https://(?:www\.|m\.)?(?:youtube\.com/(?:watch\?(?:.*&)?v=|embed/|shorts/|live/)|youtu\.be/)([A-Za-z0-9_-]{11})").captures(url).map(|m| m[1].to_string())
}

/// Le dossier a-t-il de quoi remplir l'écran médias : un titre à écouter ou une vidéo YouTube ?
pub fn has_media(d: &J) -> bool {
    !js::arr(&d["ecoute"]).is_empty() || js::arr(&d["videos"]).iter().any(|v| youtube_id(&js::gs(v, "url")).is_some())
}

/// Modèles .glb téléchargés : { fichier, licence, auteur, page }.
pub fn models(d: &J) -> Vec<J> {
    js::arr(&d["modeles3d"])
        .iter()
        .filter(|m| !js::gs(m, "fichier").is_empty())
        .map(|m| json!({ "fichier": m["fichier"], "licence": m["licence"], "auteur": m["auteur"], "page": m["url_page"] }))
        .collect()
}

/// « Pour aller plus loin » : sources, vidéos, pages des modèles 3D, lectures des sections — sans doublon.
pub fn links(d: &J) -> Vec<J> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<J> = vec![];
    let mut push = |groupe: &str, titre: String, url: String| {
        if !url.is_empty() && seen.insert(url.clone()) {
            out.push(json!({ "groupe": groupe, "titre": if titre.is_empty() { url.clone() } else { titre }, "url": url }));
        }
    };
    for s in js::arr(&d["sources"]) {
        push("Sources", js::gs(&s, "titre"), js::gs(&s, "url"));
    }
    for v in js::arr(&d["videos"]) {
        push("Vidéos", js::gs(&v, "titre"), js::gs(&v, "url"));
    }
    for m in js::arr(&d["modeles3d"]) {
        let titre = ["Modèle 3D".to_string(), js::gs(&m, "auteur"), js::gs(&m, "licence")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
        push("Modèles 3D", titre, js::gs(&m, "url_page"));
    }
    for s in js::arr(&d["sections"]) {
        for u in js::str_list(&s["sources"]) {
            push("Lectures", js::gs(&s, "titre"), u);
        }
    }
    out
}

/// Pièce désignée dans une phrase (« montre-moi le rotor principal ») : celle dont le plus de mots du nom sont dits.
pub fn piece_in(d: &J, folded: &str) -> Option<J> {
    let said: HashSet<String> = super::words(folded).into_iter().collect();
    js::arr(&d["pieces"])
        .into_iter()
        .map(|p| {
            let name = super::words(&format!("{} {}", js::gs(&p, "nom"), js::gs(&p, "id").replace('-', " ")));
            (name.iter().filter(|w| said.contains(*w)).count(), p)
        })
        .filter(|(n, _)| *n > 0)
        .max_by_key(|(n, _)| *n)
        .map(|(_, p)| p)
}

/// Commande de l'explorateur dite à voix haute (pendant une présentation ou explorateur ouvert).
pub fn control3d_of(t: &str) -> Option<&'static str> {
    let t = t.trim();
    if crate::re!(r"^(?:fais(?: le| la)? (?:fonctionner|marcher|tourner)|demarre(?: le| la)?|mets? (?:le |la )?en (?:marche|route)|montre(?: moi)? comment (?:il|elle|ca) (?:fonctionne|marche))$").is_match(t) {
        return Some("fonctionner");
    }
    if crate::re!(r"^(?:arrete(?: le| la)?(?: moteur)?|coupe(?: le| la)?(?: moteur)?|arrete de tourner)$").is_match(t) {
        return Some("arreter");
    }
    if crate::re!(r"^(?:(?:montre(?: moi)? |fais(?: moi)? |affiche(?: moi)? )?(?:la |une )?vue eclatee|eclate(?: le| la)?|demonte(?: le| la)?)$").is_match(t) {
        return Some("eclate");
    }
    if crate::re!(r"^(?:remonte(?: le| la)?|rassemble(?: le| la)?|vue normale)$").is_match(t) {
        return Some("assemble");
    }
    if crate::re!(r"^(?:(?:mode |vue )?rayons? x|transparence|vue transparente)$").is_match(t) {
        return Some("rayons");
    }
    if crate::re!(r"^(?:recentre(?: la vue| le| la)?|vue d ensemble|reinitialise(?: la vue)?|remets? la vue)$").is_match(t) {
        return Some("recentrer");
    }
    if crate::re!(r"^(?:ferme(?: la)? (?:3d|vue 3d|maquette|explorateur)|quitte(?: la)? (?:3d|vue 3d))$").is_match(t) {
        return Some("fermer");
    }
    None
}

impl Savoir {
    /// Explorateur ouvert (dossier montré), ou None.
    pub fn exploring(&self) -> Option<String> {
        self.exploring.borrow().clone()
    }

    /// Données de l'explorateur : modèles réalistes (Sketchfab), maquette, pièces, fiche, modèles .glb, liens.
    pub fn explore_data(&self, slug: &str) -> Option<J> {
        let d = self.get(slug)?;
        Some(json!({
            "slug": slug, "sujet": d["sujet"], "categorie": d["categorie"], "resume": js::slice_chars(&js::gs(&d, "resume"), 600),
            "sketchfab": self.sketchfab_models(slug), "maquette": d["maquette"], "modeles": models(&d), "liens": links(&d),
            "pieces": js::arr(&d["pieces"]).iter().map(|p| json!({ "id": p["id"], "nom": p["nom"], "description": p["description"], "position": p["position"] })).collect::<Vec<_>>(),
            "fiche": js::arr(&d["fiche"]).iter().map(|f| json!({ "cle": f["cle"], "valeur": f["valeur"], "unite": f["unite"] })).collect::<Vec<_>>(),
            "image": super::present::cover(&d)
        }))
    }

    /// Données de l'écran médias : écoute, vidéos YouTube, points forts et faibles, prix, avis, liens du profil.
    pub fn media_data(&self, slug: &str) -> Option<J> {
        let d = self.get(slug)?;
        let profil = self.profile_of(slug);
        let sujet = js::gs(&d, "sujet");
        let videos: Vec<J> = js::arr(&d["videos"]).iter().filter_map(|v| youtube_id(&js::gs(v, "url")).map(|id| json!({ "titre": v["titre"], "duree": v["duree"], "url": v["url"], "yt": id }))).collect();
        let ecoute: Vec<J> = js::arr(&d["ecoute"]).into_iter().map(|mut e| {
            e["yt"] = json!(youtube_id(&js::gs(&e, "youtube")));
            e
        }).collect();
        Some(json!({
            "slug": slug, "sujet": sujet, "categorie": d["categorie"], "resume": js::slice_chars(&js::gs(&d, "resume"), 600),
            "profil": { "id": profil["id"], "nom": profil["nom"], "icone": profil["icone"] }, "image": super::present::cover(&d),
            "ecoute": ecoute, "videos": videos, "points_forts": d["points_forts"], "points_faibles": d["points_faibles"],
            "prix": d["prix"], "avis": d["avis"], "liens": links(&d), "boutons": super::profils::buttons(&profil, &sujet)
        }))
    }

    /// Ouvre l'écran médias dans les fenêtres (`presentation` : id de la présentation qui l'ouvre).
    pub fn open_media(&self, slug: &str, presentation: Option<&str>) -> J {
        let Some(data) = self.media_data(slug) else { return json!({ "ok": false, "say": "Dossier introuvable." }) };
        let sujet = js::gs(&data, "sujet");
        if !has_media(&self.get(slug).unwrap_or_default()) {
            return json!({ "ok": false, "say": format!("Le dossier {sujet} n'a encore rien à écouter ni à regarder.") });
        }
        *self.exploring.borrow_mut() = Some(slug.to_string());
        let o = self.o.up();
        o.emit(js::spread(&json!({ "type": "savoir_media", "presentation": presentation }), &data));
        o.record("savoir_media", json!({ "dossier": slug }));
        json!({ "ok": true, "say": format!("Voici de quoi écouter et regarder {sujet}."), "display": format!("Médias · {sujet}") })
    }

    /// Écoute d'un titre du dossier dans Spotify (lecture de Jarvis : application, catalogue public ou recherche).
    pub async fn listen(&self, query: &str) -> J {
        let q = js::slice_chars(query.trim(), 200);
        if q.is_empty() {
            return json!({ "ok": false, "say": "Rien à écouter." });
        }
        let r = self.o.up().play_on_spotify(&q).await;
        json!({ "ok": js::gb(&r, "ok"), "say": if js::gb(&r, "ok") { format!("Je lance {q} sur Spotify.") } else { js::gs(&r, "message") } })
    }

    /// Fin de présentation du profil hors présentation (bouton de la card, voix) : explorateur 3D, sinon écran médias.
    pub async fn open_space(&self, slug: &str) -> J {
        let profil = self.ensure_profile(slug).await;
        if js::gs(&profil, "finale") == "media" || !js::gb(&profil, "troisD") {
            return self.open_media(slug, None);
        }
        self.explore(slug, None).await
    }

    /// Ouvre l'explorateur dans les fenêtres (`presentation` : id de la présentation qui l'ouvre). Cherche d'abord
    /// un modèle réaliste s'il n'y en a pas en cache.
    pub async fn explore(&self, slug: &str, presentation: Option<&str>) -> J {
        let found = self.find_models(slug).await;
        let Some(data) = self.explore_data(slug) else { return json!({ "ok": false, "say": "Dossier introuvable." }) };
        let sujet = js::gs(&data, "sujet");
        let mut d = self.get(slug).unwrap_or_default();
        d["sketchfab"] = json!(found);
        if !has_3d(&d) {
            return json!({ "ok": false, "say": format!("Je n'ai trouvé aucun modèle 3D de {sujet}.") });
        }
        *self.exploring.borrow_mut() = Some(slug.to_string());
        let o = self.o.up();
        o.emit(js::spread(&json!({ "type": "savoir_3d", "presentation": presentation }), &data));
        o.record("savoir_3d", json!({ "dossier": slug }));
        json!({ "ok": true, "say": format!("Voici {sujet} en 3D. Tourne autour, clique une pièce, ou demande-moi de le faire fonctionner."), "display": format!("3D · {sujet}") })
    }

    /// Fermeture de l'explorateur (HUD, voix, arrêt général). Celle qui termine une présentation la clôt aussi.
    pub fn close_explore(&self, from_hud: bool) {
        if self.exploring.borrow_mut().take().is_none() {
            return;
        }
        if !from_hud {
            self.o.up().emit(json!({ "type": "savoir_3d_close" }));
            self.o.up().emit(json!({ "type": "savoir_media_close" }));
        }
        if self.presenting() {
            self.end_presentation(true);
        }
    }

    /// Modèle .glb d'un dossier en adresse data: (le HUD ne lit jamais le disque).
    pub fn model(&self, slug: &str, file: &str) -> Option<String> {
        if !crate::re!(r"^modele-[0-9a-f]{16}\.glb$").is_match(file) || !self.dossiers.borrow().contains_key(slug) {
            return None;
        }
        let path = self.dir.join(slug).join(file);
        if std::fs::metadata(&path).ok()?.len() > GLB_VIEW_MAX {
            return None;
        }
        let bytes = std::fs::read(path).ok()?;
        Some(format!("data:model/gltf-binary;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    }

    /// Commande de l'explorateur : diffusée au HUD. false si l'explorateur est fermé.
    pub fn control3d(&self, action: &str) -> bool {
        if self.exploring().is_none() {
            return false;
        }
        if action == "fermer" {
            self.close_explore(false);
            return true;
        }
        self.o.up().emit(json!({ "type": "savoir_3d_control", "action": action }));
        true
    }

    /// Phrase dite explorateur ouvert (hors présentation) : commande, pièce à montrer, ou question sur le sujet.
    pub fn on_explore_utterance(&self, folded: &str, raw: &str, routed_locally: bool) -> bool {
        let Some(slug) = self.exploring() else { return false };
        let o = self.o.up();
        if let Some(c) = control3d_of(folded) {
            self.control3d(c);
            return true;
        }
        if crate::re!(r"^(?:stop|arrete|ferme|c est bon merci|merci c est tout|ca suffit)$").is_match(folded.trim()) {
            self.close_explore(false);
            return true;
        }
        if self.show_piece(&slug, folded) {
            return true;
        }
        if routed_locally {
            return false;
        }
        let question = raw.trim().to_string();
        let w = self.o.clone();
        o.emit(json!({ "type": "voice_stop" }));
        o.emit(json!({ "type": "savoir_3d_control", "action": "question", "question": question }));
        js::spawn(async move {
            if let Some(o) = w.upgrade() {
                o.savoir.ask(None, &slug, &question).await;
            }
        });
        true
    }

    /// « Montre-moi le rotor », « c'est quoi le fenestron » : la caméra vole vers la pièce, sa description est dite.
    pub fn show_piece(&self, slug: &str, folded: &str) -> bool {
        let Some(m) = crate::re!(r"^(?:montre|zoome? sur|focus sur|selectionne|va sur|c est quoi|qu est ce que c est que|explique(?: moi)?)(?: moi)? (?:le |la |les |l |un |une )?(.+)$").captures(folded.trim()) else { return false };
        let Some(d) = self.get(slug) else { return false };
        let Some(p) = piece_in(&d, &m[1]) else { return false };
        let o = self.o.up();
        o.emit(json!({ "type": "savoir_3d_control", "action": "piece", "piece": p["id"] }));
        let desc = js::gs(&p, "description");
        let text = if desc.is_empty() { format!("Voici : {}.", js::gs(&p, "nom")) } else { format!("{} : {}", js::gs(&p, "nom"), js::slice_chars(&desc, 400)) };
        o.emit(json!({ "type": "say", "text": text }));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier() -> J {
        json!({
            "sujet": "Airbus H160",
            "pieces": [{ "id": "rotor-principal", "nom": "Rotor principal", "description": "Cinq pales Blue Edge." }, { "id": "fenestron", "nom": "Fenestron" }],
            "sources": [{ "titre": "Wikipédia", "url": "https://fr.wikipedia.org/wiki/H160" }],
            "videos": [{ "titre": "Vol", "url": "https://www.youtube.com/watch?v=a" }],
            "sections": [{ "titre": "Conception", "sources": ["https://fr.wikipedia.org/wiki/H160", "https://www.aerobuzz.fr/h160"] }],
            "modeles3d": [{ "url_page": "https://nasa.gov/m", "auteur": "NASA", "licence": "domaine public" }],
            "maquette": { "elements": [{ "forme": "boite" }] }
        })
    }

    #[test]
    fn liens_regroupes_sans_doublon() {
        let l = links(&dossier());
        let urls: Vec<String> = l.iter().map(|x| js::gs(x, "url")).collect();
        assert_eq!(urls, ["https://fr.wikipedia.org/wiki/H160", "https://www.youtube.com/watch?v=a", "https://nasa.gov/m", "https://www.aerobuzz.fr/h160"]);
        assert_eq!(l[2]["titre"], "Modèle 3D · NASA · domaine public");
        assert_eq!(l[3]["groupe"], "Lectures");
        assert!(has_3d(&dossier()));
        assert!(!has_3d(&json!({ "maquette": null, "modeles3d": [{ "url_glb": "https://x/y.glb" }] })), "un modèle non téléchargé ne compte pas");
    }

    #[test]
    fn pieces_et_commandes_a_la_voix() {
        let d = dossier();
        assert_eq!(piece_in(&d, "le rotor").unwrap()["id"], "rotor-principal");
        assert_eq!(piece_in(&d, "fenestron").unwrap()["id"], "fenestron");
        assert!(piece_in(&d, "la cabine").is_none());
        assert_eq!(control3d_of("fais le fonctionner"), Some("fonctionner"));
        assert_eq!(control3d_of("montre moi la vue eclatee"), Some("eclate"));
        assert_eq!(control3d_of("rayons x"), Some("rayons"));
        assert_eq!(control3d_of("ferme la 3d"), Some("fermer"));
        assert_eq!(control3d_of("quelle est sa vitesse"), None);
    }

    #[test]
    fn videos_youtube_et_ecran_medias() {
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://youtu.be/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://www.youtube.com/live/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert!(youtube_id("https://vimeo.com/123").is_none());
        assert!(!has_media(&dossier()), "une vidéo hors YouTube ne suffit pas");
        assert!(has_media(&json!({ "ecoute": [{ "titre": "Discovery" }] })));
    }
}
