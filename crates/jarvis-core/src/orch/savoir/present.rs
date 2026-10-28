// Présentation animée d'un dossier « Savoir » (« présente-moi le H160 ») : Claude Haiku écrit le script (une
// narration par scène et les références aux données du dossier), le noyau le vérifie contre le dossier — aucun
// chiffre, image ou date affiché ne vient d'ailleurs — puis le HUD joue les scènes dans le cœur (ui/presenter.js),
// présentées par le personnage. Le HUD demande chaque scène (op « scene ») : le noyau la diffuse à toutes les
// fenêtres et dit sa narration. Pendant la présentation, « suivant », « reviens », « pause », « stop » la pilotent
// et une question libre reçoit une réponse tirée du RAG du dossier, avec l'image ou la pièce à montrer.
// Le profil du dossier (profils.rs) guide le script — scènes conseillées : points forts et faibles, prix, avis,
// écoute… — et sa fin : explorateur 3D (scène « exploration », explore.rs), écran médias (« media » : écoute et
// vidéos) ou liens. L'explorateur et l'écran médias restent ouverts, présentation active, jusqu'à leur fermeture.
// Le script est préparé dès la fin de la recherche (prepare) et gardé tant que le dossier ne change pas.

use std::collections::HashSet;

use serde_json::{json, Value};

use super::dossier::ident;
use super::Savoir;
use crate::js::{self, J};
use crate::orch::Up;

pub const MAX_SCENES: usize = 14;
pub const TYPES: &[&str] = &["titre", "image", "chiffres", "fiche", "chronologie", "comparaison", "piece", "video", "conclusion", "exploration", "atouts", "prix", "avis", "ecoute", "liens", "media"];
/// Scènes de fin (ajoutées selon le profil, jamais au milieu).
const FINALS: &[&str] = &["exploration", "media"];

pub struct Presentation {
    pub id: String,
    pub slug: String,
    pub scenes: Vec<J>,
    pub current: usize,
    pub paused: bool,
}

const SCRIPT_PROMPT: &str = "Tu écris le script d'une présentation animée qu'un assistant vocal va dire à voix haute, scène par scène, à partir du dossier JSON fourni. Réponds uniquement par un objet JSON {\"scenes\":[…]}, sans texte autour. 7 à 12 scènes, dans un ordre qui raconte le sujet : d'abord un titre, ensuite ce qui le rend intéressant, et une conclusion à la fin. Chaque scène a \"type\" et \"dit\" (la narration : une à trois phrases en français, 300 caractères au plus, sans markdown, sans liste, naturelle à l'oral, fidèle au dossier). Types : {\"type\":\"titre\"} ; {\"type\":\"image\",\"image\":\"<fichier exact d'une image>\"} ; {\"type\":\"chiffres\",\"cles\":[\"<clé exacte de la fiche>\", … 4 au plus]} ; {\"type\":\"fiche\",\"cles\":[… 8 au plus]} ; {\"type\":\"chronologie\",\"dates\":[\"<date exacte>\", … 6 au plus]} ; {\"type\":\"comparaison\",\"autre\":\"<nom exact>\"} ; {\"type\":\"piece\",\"piece\":\"<id exact d'une pièce>\",\"image\":\"<fichier facultatif>\"} ; {\"type\":\"video\",\"url\":\"<adresse exacte>\"} ; {\"type\":\"atouts\"} (points forts et points faibles du dossier) ; {\"type\":\"prix\"} (prix relevés) ; {\"type\":\"avis\"} (notes des tests ou de la critique) ; {\"type\":\"ecoute\",\"titre\":\"<titre exact d'un élément de ecoute>\"} (un album, un titre ou un concert à écouter) ; {\"type\":\"liens\"} (où en savoir plus) ; {\"type\":\"conclusion\"}. Le champ profil donne le genre du dossier et scenes_conseillees l'ordre à suivre quand le dossier a de quoi remplir ces scènes : pour un produit, parle des points forts, des points faibles et du prix ; pour de la musique, de ce qu'il faut écouter. N'utilise que des fichiers, clés, dates, noms, ids, titres et adresses présents dans le dossier ; varie les types ; n'invente aucun chiffre ni aucun prix.";

const ASK_PROMPT: &str = "Pendant une présentation animée, l'utilisateur pose une question à voix haute. Réponds uniquement à partir des extraits du dossier fournis, en français, en une à trois phrases courtes faites pour l'oral (sans markdown) ; si les extraits ne suffisent pas, dis-le simplement. Réponds uniquement par un objet JSON {\"reponse\":\"…\",\"image\":\"<fichier exact d'une image à montrer, ou null>\",\"piece\":\"<id exact d'une pièce à montrer, ou null>\"}. Choisis une image ou une pièce seulement si elle illustre vraiment la réponse (la question « montre-moi le cockpit » appelle l'image du cockpit).";

fn txt(v: &J, max: usize) -> String {
    let s = v.as_str().unwrap_or("");
    js::slice_chars(crate::re!(r"\s+").replace_all(s, " ").trim(), max)
}

fn key(s: &str) -> String {
    ident(s, 200)
}

/// Images téléchargées : (fichier, vue, légende).
fn images(d: &J) -> Vec<(String, String, String)> {
    d["images"].as_array().map(|a| a.iter().filter(|i| !js::gs(i, "fichier").is_empty()).map(|i| (js::gs(i, "fichier"), js::gs(i, "vue"), js::gs(i, "legende"))).collect()).unwrap_or_default()
}

/// Image de couverture : une vue de profil, sinon la première.
pub fn cover(d: &J) -> Option<String> {
    let imgs = images(d);
    imgs.iter().find(|(_, v, _)| v == "profil").or(imgs.first()).map(|(f, _, _)| f.clone())
}

fn fiche_rows(d: &J, keys: &[String], max: usize) -> Vec<J> {
    let fiche = d["fiche"].as_array().cloned().unwrap_or_default();
    let mut out: Vec<J> = vec![];
    for k in keys {
        if let Some(f) = fiche.iter().find(|f| key(&js::gs(f, "cle")) == key(k)) {
            if !out.iter().any(|x| x["cle"] == f["cle"]) {
                out.push(json!({ "cle": f["cle"], "valeur": f["valeur"], "unite": f["unite"] }));
            }
        }
    }
    out.truncate(max);
    out
}

/// Scène écrite par l'IA → scène affichable, construite avec les seules données du dossier. None si elle ne tient pas.
pub fn resolve_scene(raw: &J, d: &J) -> Option<J> {
    let kind = js::gs(raw, "type");
    if !TYPES.contains(&kind.as_str()) {
        return None;
    }
    let dit = txt(&raw["dit"], 420);
    if dit.chars().count() < 8 {
        return None;
    }
    let imgs = images(d);
    let img_ok = |f: &str| imgs.iter().find(|(x, _, _)| x == f).cloned();
    let sujet = js::gs(d, "sujet");
    let mut scene = match kind.as_str() {
        "titre" | "conclusion" => json!({ "titre": sujet, "sousTitre": d["categorie"], "image": cover(d) }),
        "exploration" if super::explore::has_3d(d) => json!({ "titre": sujet, "image": cover(d) }),
        "image" => {
            let (f, vue, leg) = img_ok(&js::gs(raw, "image"))?;
            json!({ "image": f, "vue": vue, "legende": leg })
        }
        "chiffres" | "fiche" => {
            let max = if kind == "chiffres" { 4 } else { 8 };
            let rows = fiche_rows(d, &js::str_list(&raw["cles"]), max);
            if rows.is_empty() || (kind == "fiche" && rows.len() < 2) {
                return None;
            }
            json!({ "lignes": rows })
        }
        "chronologie" => {
            let all = d["chronologie"].as_array().cloned().unwrap_or_default();
            let wanted: Vec<String> = js::str_list(&raw["dates"]).iter().map(|x| key(x)).collect();
            let mut items: Vec<J> = all.iter().filter(|c| wanted.contains(&key(&js::gs(c, "date")))).take(6).cloned().collect();
            if items.len() < 2 {
                items = all.iter().take(6).cloned().collect();
            }
            if items.len() < 2 {
                return None;
            }
            json!({ "items": items })
        }
        "comparaison" => {
            let c = d["comparaisons"].as_array()?.iter().find(|c| key(&js::gs(c, "autre")) == key(&js::gs(raw, "autre")))?.clone();
            json!({ "sujet": sujet, "autre": c["autre"], "points": js::str_list(&c["points"]).into_iter().take(4).collect::<Vec<_>>() })
        }
        "piece" => {
            let p = d["pieces"].as_array()?.iter().find(|p| js::gs(p, "id") == ident(&js::gs(raw, "piece"), 40))?.clone();
            let image = img_ok(&js::gs(raw, "image")).map(|(f, _, _)| f).or_else(|| imgs.iter().find(|(_, v, _)| v == "detail").map(|(f, _, _)| f.clone())).or_else(|| cover(d));
            json!({ "piece": p["id"], "nom": p["nom"], "description": js::slice_chars(&js::gs(&p, "description"), 220), "position": p["position"], "image": image })
        }
        "video" => {
            let v = d["videos"].as_array()?.iter().find(|v| js::gs(v, "url") == js::gs(raw, "url"))?.clone();
            json!({ "titre": v["titre"], "url": v["url"], "duree": v["duree"], "image": cover(d) })
        }
        "atouts" => {
            let (forts, faibles) = (js::str_list(&d["points_forts"]), js::str_list(&d["points_faibles"]));
            if forts.len() + faibles.len() < 2 {
                return None;
            }
            json!({ "forts": forts.into_iter().take(4).collect::<Vec<_>>(), "faibles": faibles.into_iter().take(4).collect::<Vec<_>>() })
        }
        "prix" => {
            let mut rows: Vec<J> = js::arr(&d["prix"]).into_iter().filter(|p| p["prix"].is_number()).collect();
            if rows.is_empty() {
                return None;
            }
            rows.sort_by(|a, b| a["prix"].as_f64().partial_cmp(&b["prix"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
            json!({ "lignes": rows.into_iter().take(5).map(|p| json!({ "vendeur": p["vendeur"], "prix": p["prix"], "devise": p["devise"], "date": p["date"] })).collect::<Vec<_>>() })
        }
        "avis" => {
            let rows: Vec<J> = js::arr(&d["avis"]).into_iter().take(3).map(|a| json!({ "source": a["source"], "note": a["note"], "sur": a["sur"], "resume": js::slice_chars(&js::gs(&a, "resume"), 160) })).collect();
            if rows.is_empty() {
                return None;
            }
            json!({ "lignes": rows })
        }
        "ecoute" => {
            let all = js::arr(&d["ecoute"]);
            let e = all.iter().find(|e| key(&js::gs(e, "titre")) == key(&js::gs(raw, "titre"))).or(all.first())?.clone();
            json!({ "titre": e["titre"], "artiste": e["artiste"], "genre": e["type"], "annee": e["annee"], "spotify": e["spotify"], "youtube": e["youtube"], "image": cover(d) })
        }
        "liens" => {
            let liens: Vec<J> = super::explore::links(d).into_iter().take(6).collect();
            let boutons = super::profils::buttons(&d["profil_def"], &sujet);
            if liens.is_empty() && boutons.is_empty() {
                return None;
            }
            json!({ "liens": liens, "boutons": boutons })
        }
        "media" if super::explore::has_media(d) => json!({ "titre": sujet, "image": cover(d) }),
        _ => return None,
    };
    scene["type"] = json!(kind);
    scene["dit"] = json!(dit);
    Some(scene)
}

/// Fin de présentation du dossier : celle de son profil (`profil_def`, ajouté par l'appelant) ; un dossier sans
/// profil finit par l'explorateur 3D s'il a de quoi.
fn finale_of(d: &J) -> String {
    Some(js::gs(&d["profil_def"], "finale")).filter(|f| !f.is_empty()).unwrap_or_else(|| "3d".into())
}

/// Script de l'IA → scènes vérifiées (doublons d'image retirés, titre en tête). La fin dépend du profil : explorateur
/// 3D (« exploration ») ou écran médias (« media ») ajoutés au besoin après la conclusion, ou liens avant elle.
pub fn resolve_script(raw: &J, d: &J) -> Vec<J> {
    let mut seen_images: HashSet<String> = HashSet::new();
    let mut out: Vec<J> = vec![];
    let mut finals: Vec<J> = vec![];
    for s in raw["scenes"].as_array().into_iter().flatten() {
        let Some(scene) = resolve_scene(s, d) else { continue };
        let t = js::gs(&scene, "type");
        if FINALS.contains(&t.as_str()) {
            finals.push(scene);
            continue;
        }
        if t == "image" && !seen_images.insert(js::gs(&scene, "image")) {
            continue;
        }
        if (t == "titre" && !out.is_empty()) || out.len() >= MAX_SCENES {
            continue;
        }
        out.push(scene);
    }
    if out.is_empty() {
        return out;
    }
    let sujet = js::gs(d, "sujet");
    let mine = |t: &str| finals.iter().find(|s| js::gs(s, "type") == t).cloned();
    match finale_of(d).as_str() {
        "3d" if super::explore::has_3d(d) => {
            let default = json!({ "type": "exploration", "dit": format!("Voici {sujet} en 3D. Tourne autour, clique une pièce, ou demande-moi de le faire fonctionner.") });
            out.extend(mine("exploration").or_else(|| resolve_scene(&default, d)));
        }
        "media" if super::explore::has_media(d) => {
            let default = json!({ "type": "media", "dit": format!("Voici de quoi écouter et regarder {sujet}. Choisis un titre, ou demande-moi de le lancer.") });
            out.extend(mine("media").or_else(|| resolve_scene(&default, d)));
        }
        // Fin « liens », ou fin du profil impossible (pas de modèle 3D, rien à écouter) : les liens avant la conclusion.
        _ if !d["profil_def"].is_null() && !out.iter().any(|s| js::gs(s, "type") == "liens") => {
            let liens = resolve_scene(&json!({ "type": "liens", "dit": format!("Pour aller plus loin sur {sujet}, voici les sources et les liens utiles.") }), d);
            if let Some(l) = liens {
                let at = if js::gs(out.last().unwrap(), "type") == "conclusion" { out.len() - 1 } else { out.len() };
                out.insert(at, l);
            }
        }
        _ => {}
    }
    out
}

/// Script sans IA, dans l'ordre des scènes conseillées par le profil (titre, image, chiffres, chronologie,
/// comparaison, pièce et conclusion sans profil) — seulement ce que le dossier peut remplir.
pub fn fallback_script(d: &J) -> Vec<J> {
    let sujet = js::gs(d, "sujet");
    let resume = js::gs(d, "resume");
    let first = |text: &str| -> String {
        let s = crate::re!(r"^(.{20,300}?[.!?])(?:\s|$)").captures(text).map(|m| m[1].to_string()).unwrap_or_else(|| js::slice_chars(text, 280));
        s.trim().to_string()
    };
    let order: Vec<String> = Some(js::str_list(&d["profil_def"]["scenes"])).filter(|l| !l.is_empty()).unwrap_or_else(|| {
        ["titre", "image", "chiffres", "chronologie", "comparaison", "piece", "conclusion"].iter().map(|s| s.to_string()).collect()
    });
    let imgs = images(d);
    let mut raw: Vec<J> = vec![];
    for t in &order {
        let scene = match t.as_str() {
            "titre" => Some(json!({ "type": "titre", "dit": if resume.is_empty() { format!("Voici {sujet}.") } else { first(&resume) } })),
            "image" => imgs.iter().find(|(f, _, _)| Some(f.clone()) != cover(d)).or(imgs.first()).map(|(f, _, leg)| json!({ "type": "image", "image": f, "dit": if leg.is_empty() { format!("Voici {sujet} en image.") } else { format!("{leg}.") } })),
            "chiffres" | "fiche" => {
                let keys: Vec<String> = js::arr(&d["fiche"]).iter().map(|f| js::gs(f, "cle")).collect();
                let rows = fiche_rows(d, &keys, 4);
                let spoken: Vec<String> = rows.iter().map(|r| format!("{} : {}{}", js::gs(r, "cle"), js::gs(r, "valeur"), Some(js::gs(r, "unite")).filter(|u| !u.is_empty()).map(|u| format!(" {u}")).unwrap_or_default())).collect();
                (!keys.is_empty() && t == "chiffres").then(|| json!({ "type": "chiffres", "cles": keys, "dit": format!("Quelques chiffres. {}.", spoken.join(", ")) }))
            }
            "chronologie" => (js::arr(&d["chronologie"]).len() >= 2).then(|| {
                let c = &d["chronologie"][0];
                json!({ "type": "chronologie", "dates": [], "dit": format!("Un peu d'histoire : en {}, {}.", js::gs(c, "date"), js::gs(c, "fait").trim_end_matches('.')) })
            }),
            "comparaison" => js::arr(&d["comparaisons"]).first().map(|c| json!({ "type": "comparaison", "autre": c["autre"], "dit": format!("Face à {} : {}.", js::gs(c, "autre"), js::str_list(&c["points"]).first().cloned().unwrap_or_default().trim_end_matches('.')) })),
            "piece" => js::arr(&d["pieces"]).first().map(|p| json!({ "type": "piece", "piece": p["id"], "dit": format!("{} : {}", js::gs(p, "nom"), first(&js::gs(p, "description"))) })),
            "video" => js::arr(&d["videos"]).first().map(|v| json!({ "type": "video", "url": v["url"], "dit": format!("Une vidéo à voir : {}.", js::gs(v, "titre").trim_end_matches('.')) })),
            "atouts" => {
                let (forts, faibles) = (js::str_list(&d["points_forts"]), js::str_list(&d["points_faibles"]));
                let mut dit = vec![];
                if let Some(f) = forts.first() {
                    dit.push(format!("Son point fort : {}.", f.trim_end_matches('.')));
                }
                if let Some(f) = faibles.first() {
                    dit.push(format!("Son point faible : {}.", f.trim_end_matches('.')));
                }
                Some(json!({ "type": "atouts", "dit": dit.join(" ") }))
            }
            "prix" => js::arr(&d["prix"]).iter().filter(|p| p["prix"].is_number()).min_by(|a, b| a["prix"].as_f64().partial_cmp(&b["prix"].as_f64()).unwrap_or(std::cmp::Ordering::Equal)).map(|p| {
                json!({ "type": "prix", "dit": format!("Côté prix : à partir de {} {} chez {}.", price(&p["prix"]), js::gs(p, "devise"), js::gs(p, "vendeur")) })
            }),
            "avis" => js::arr(&d["avis"]).first().map(|a| {
                let note = Some(js::gs(a, "note")).filter(|n| !n.is_empty()).map(|n| format!(" lui donne {n}{}", Some(js::gs(a, "sur")).filter(|s| !s.is_empty()).map(|s| format!(" sur {s}")).unwrap_or_default())).unwrap_or_else(|| " en dit".into());
                json!({ "type": "avis", "dit": format!("{}{note} : {}", js::gs(a, "source"), first(&js::gs(a, "resume"))) })
            }),
            "ecoute" => js::arr(&d["ecoute"]).first().map(|e| json!({ "type": "ecoute", "titre": e["titre"], "dit": format!("À écouter : {}{}.", js::gs(e, "titre"), Some(js::gs(e, "artiste")).filter(|a| !a.is_empty()).map(|a| format!(", de {a}")).unwrap_or_default()) })),
            "liens" => Some(json!({ "type": "liens", "dit": format!("Pour aller plus loin sur {sujet}, voici les sources et les liens utiles.") })),
            "conclusion" => Some(json!({ "type": "conclusion", "dit": format!("Voilà pour {sujet}. Tu peux me poser une question.") })),
            _ => None,
        };
        raw.extend(scene);
    }
    resolve_script(&json!({ "scenes": raw }), d)
}

/// Prix lisible à l'oral : « 699 », « 12,50 ».
pub fn price(v: &J) -> String {
    let n = v.as_f64().unwrap_or(0.0);
    if (n - n.round()).abs() < 0.005 { format!("{}", n.round() as i64) } else { format!("{n:.2}").replace('.', ",") }
}

/// Dossier résumé pour la consigne (taille bornée).
fn digest(d: &J) -> J {
    json!({
        "sujet": d["sujet"], "categorie": d["categorie"], "resume": js::slice_chars(&js::gs(d, "resume"), 1500),
        "fiche": d["fiche"].as_array().map(|a| a.iter().take(40).map(|f| json!({ "cle": f["cle"], "valeur": f["valeur"], "unite": f["unite"] })).collect::<Vec<_>>()),
        "sections": d["sections"].as_array().map(|a| a.iter().take(12).map(|s| json!({ "titre": s["titre"], "texte": js::slice_chars(&js::gs(s, "texte"), 600) })).collect::<Vec<_>>()),
        "chronologie": d["chronologie"].as_array().map(|a| a.iter().take(20).cloned().collect::<Vec<_>>()),
        "comparaisons": d["comparaisons"],
        "pieces": d["pieces"].as_array().map(|a| a.iter().map(|p| json!({ "id": p["id"], "nom": p["nom"], "description": js::slice_chars(&js::gs(p, "description"), 200) })).collect::<Vec<_>>()),
        "images": images(d).iter().map(|(f, v, l)| json!({ "fichier": f, "vue": v, "legende": l })).collect::<Vec<_>>(),
        "videos": d["videos"].as_array().map(|a| a.iter().map(|v| json!({ "url": v["url"], "titre": v["titre"] })).collect::<Vec<_>>()),
        "points_forts": d["points_forts"], "points_faibles": d["points_faibles"],
        "prix": js::arr(&d["prix"]).iter().map(|p| json!({ "vendeur": p["vendeur"], "prix": p["prix"], "devise": p["devise"] })).collect::<Vec<_>>(),
        "avis": js::arr(&d["avis"]).iter().map(|a| json!({ "source": a["source"], "note": a["note"], "sur": a["sur"], "resume": js::slice_chars(&js::gs(a, "resume"), 200) })).collect::<Vec<_>>(),
        "ecoute": js::arr(&d["ecoute"]).iter().map(|e| json!({ "titre": e["titre"], "type": e["type"], "artiste": e["artiste"] })).collect::<Vec<_>>(),
        "profil": d["profil_def"]["nom"], "scenes_conseillees": d["profil_def"]["scenes"]
    })
}

/// Appel rapide sans outil (Haiku ou repli AGY Flash Low) : `input` sur l'entrée standard, réponse texte. None hors ligne, en test ou en échec.
pub(super) async fn haiku(o: &crate::orch::Orchestrator, system: &str, input: &str, label: &str) -> Option<String> {
    if js::env("JARVIS_TEST").is_some() {
        return None;
    }
    let task_id = format!("pres{}", js::now_ms());
    let (first_engine, _, _) = o.first_available_ai_slot();
    o.emit(json!({ "type": "create", "id": task_id, "label": label, "detail": if first_engine == "agy" { "Flash Low" } else { "Haiku" } }));
    use tokio::io::AsyncWriteExt;

    let try_claude = async {
        let claude = o.claude_path().ok_or_else(|| "claude absent".to_string())?;
        let mut cmd = tokio::process::Command::new(&claude);
        cmd.args([
            "-p", "--output-format", "text", "--tools", "", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}",
            "--permission-mode", "manual", "--no-session-persistence", "--system-prompt", system, "--model", "haiku",
        ])
        .current_dir(crate::paths::workspace())
        .env("JARVIS_TASK_ID", &task_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes()).await;
        }
        let out = tokio::time::timeout(std::time::Duration::from_secs(90), child.wait_with_output()).await.map_err(|_| "délai dépassé".to_string())?.map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !stdout.is_empty() {
            Ok(stdout)
        } else {
            Err("réponse vide".into())
        }
    };

    let try_agy = async {
        if o.agy.is_empty() || o.agy_blocked() {
            return Err("agy indisponible".to_string());
        }
        let mut cmd = tokio::process::Command::new(&o.agy);
        cmd.args([
            "-p", "--output-format", "text", "--mode", "plan",
            "--model", "gemini-3.8-flash-low", "--effort", "low",
        ])
        .current_dir(crate::paths::workspace())
        .env("JARVIS_TASK_ID", &task_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            let full = format!("[Consignes : {system}]\n\n{input}");
            let _ = stdin.write_all(full.as_bytes()).await;
        }
        let out = tokio::time::timeout(std::time::Duration::from_secs(90), child.wait_with_output()).await.map_err(|_| "délai dépassé".to_string())?.map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !stdout.is_empty() {
            Ok(stdout)
        } else {
            Err("réponse vide".into())
        }
    };

    let res = if first_engine == "agy" {
        match try_agy.await {
            Ok(s) => Ok(s),
            Err(_) => try_claude.await,
        }
    } else {
        match try_claude.await {
            Ok(s) => Ok(s),
            Err(_) => {
                o.emit(json!({ "type": "update", "id": task_id, "detail": "bascule sur AGY Flash Low…" }));
                try_agy.await
            }
        }
    };
    o.emit(json!({ "type": "finish", "id": task_id, "status": if res.is_ok() { "done" } else { "error" } }));
    res.ok()
}

/// Commande de pilotage dite pendant une présentation.
pub fn control_of(t: &str) -> Option<&'static str> {
    let t = t.trim();
    if crate::re!(r"^(?:(?:scene |diapo )?suivante?|continue|continuer|la suite|passe(?: a la suite| a la scene suivante)?|ensuite|vas y|reprends|on continue)$").is_match(t) {
        return Some("suivant");
    }
    if crate::re!(r"^(?:(?:scene |diapo )?precedente?|reviens(?: en arriere)?|retour(?: en arriere)?|recule|reviens a la scene precedente)$").is_match(t) {
        return Some("precedent");
    }
    if crate::re!(r"^(?:pause|attends|mets? (?:la presentation )?en pause|une seconde|stop une seconde)$").is_match(t) {
        return Some("pause");
    }
    if crate::re!(r"^(?:stop|arrete|termine|fin de la presentation|arrete la presentation|ferme la presentation|c est bon merci|merci c est tout|ca suffit)$").is_match(t) {
        return Some("stop");
    }
    if crate::re!(r"^(?:recommence|reprends du debut|depuis le debut)$").is_match(t) {
        return Some("debut");
    }
    if crate::re!(r"^(?:ecoute(?: le| la| ca| l album| ce titre)?|lance(?: le| la| l album| ce titre)?|mets(?: le| la| l album)?|joue(?: le| la)?|fais(?: moi)? ecouter(?: ca)?)$").is_match(t) {
        return Some("ecouter");
    }
    None
}

impl Savoir {
    pub fn presenting(&self) -> bool {
        self.presentation.borrow().is_some()
    }

    /// « Présente-moi le H160 » : script, puis diffusion aux fenêtres (le HUD lance la première scène).
    pub async fn present(&self, slug: &str) -> J {
        let o = self.o.up();
        let Some(d) = self.get(slug) else { return json!({ "ok": false, "say": "Dossier introuvable." }) };
        if self.is_running(slug) && d["sections"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
            return json!({ "ok": false, "say": "Le dossier est encore en préparation : attends la fin de la recherche." });
        }
        let sujet = js::gs(&d, "sujet");
        let scenes = self.script(slug).await;
        if scenes.len() < 2 {
            return json!({ "ok": false, "say": format!("Le dossier sur {sujet} est trop mince pour une présentation : approfondis-le d'abord.") });
        }
        let id = format!("p{}", js::now_ms());
        let files: Vec<String> = scenes.iter().map(|s| js::gs(s, "image")).filter(|f| !f.is_empty()).collect::<HashSet<_>>().into_iter().collect();
        *self.presentation.borrow_mut() = Some(Presentation { id: id.clone(), slug: slug.to_string(), scenes: scenes.clone(), current: 0, paused: false });
        if let Some(dm) = self.dossiers.borrow_mut().get_mut(slug) {
            dm["meta"]["presentations"] = json!(dm["meta"]["presentations"].as_i64().unwrap_or(0) + 1);
        }
        self.save(slug);
        o.record("savoir_presentation", json!({ "dossier": slug, "scenes": scenes.len() }));
        o.emit(json!({
            "type": "presentation", "id": id, "slug": slug, "sujet": sujet, "scenes": scenes, "images": files,
            "personnage": o.status.d.borrow().heart_style.clone()
        }));
        json!({ "ok": true, "display": format!("Présentation · {sujet}"), "quiet": true })
    }

    /// Script de la présentation : celui préparé à la fin de la recherche s'il est à jour, sinon écrit maintenant (Haiku,
    /// modèle 3D cherché en même temps pour un profil 3D) et gardé dans meta.script.
    pub async fn script(&self, slug: &str) -> Vec<J> {
        let Some(d) = self.get(slug) else { return vec![] };
        let maj = js::gs(&d["meta"], "maj");
        let cached = &d["meta"]["script"];
        if !maj.is_empty() && js::gs(cached, "maj") == maj && cached["scenes"].as_array().map(|a| a.len() >= 2).unwrap_or(false) {
            return js::arr(&cached["scenes"]);
        }
        let o = self.o.up();
        let profil = self.ensure_profile(slug).await;
        let mut d = self.get(slug).unwrap_or(d);
        d["profil_def"] = profil.clone();
        let input = serde_json::to_string(&digest(&d)).unwrap_or_default();
        let label = format!("Présentation · {}", js::gs(&d, "sujet"));
        let models = async {
            if js::gb(&profil, "troisD") { self.find_models(slug).await } else { vec![] }
        };
        let (text, models) = tokio::join!(haiku(&o, SCRIPT_PROMPT, &input, &label), models);
        d["sketchfab"] = json!(models);
        let script = match text.and_then(|t| super::dossier::parse_loose(&t)) {
            Some(raw) => resolve_script(&raw, &d),
            None => vec![],
        };
        let scenes = if script.len() >= 3 { script } else { fallback_script(&d) };
        if scenes.len() >= 2 && !maj.is_empty() {
            if let Some(dm) = self.dossiers.borrow_mut().get_mut(slug) {
                dm["meta"]["script"] = json!({ "maj": maj, "scenes": scenes });
            }
            self.save(slug);
        }
        scenes
    }

    /// Fin de recherche : profil, modèle 3D réaliste (profil 3D) et présentation préparés, pour qu'ils soient prêts.
    pub async fn prepare(&self, slug: &str) -> J {
        let profil = self.ensure_profile(slug).await;
        let scenes = self.script(slug).await;
        let models = if js::gb(&profil, "troisD") { self.find_models(slug).await.len() } else { 0 };
        json!({ "profil": profil["nom"], "scenes": scenes.len(), "modeles3d": models, "finale": scenes.last().map(|s| s["type"].clone()) })
    }

    /// Le HUD demande la scène n : diffusée à toutes les fenêtres, narration dite. Au-delà de la dernière : fin.
    pub fn scene(&self, id: &str, n: i64) -> J {
        let o = self.o.up();
        let (dit, total, explore) = {
            let mut p = self.presentation.borrow_mut();
            let Some(p) = p.as_mut().filter(|p| p.id == id) else { return json!({ "ok": false }) };
            if n < 0 || n as usize >= p.scenes.len() {
                (None, p.scenes.len(), None)
            } else {
                p.current = n as usize;
                p.paused = false;
                let scene = &p.scenes[n as usize];
                let t = js::gs(scene, "type");
                (Some(js::gs(scene, "dit")), p.scenes.len(), FINALS.contains(&t.as_str()).then(|| (p.slug.clone(), t)))
            }
        };
        if let Some((slug, kind)) = explore.filter(|(s, _)| self.exploring().as_deref() != Some(s.as_str())) {
            if kind == "media" {
                self.open_media(&slug, Some(id));
            } else {
                let (w, id) = (self.o.clone(), id.to_string());
                js::spawn(async move {
                    if let Some(o) = w.upgrade() {
                        o.savoir.explore(&slug, Some(&id)).await;
                    }
                });
            }
        }
        match dit {
            Some(text) => {
                o.emit(json!({ "type": "presentation_scene", "id": id, "n": n, "total": total }));
                o.emit(json!({ "type": "say", "text": text, "presentation": id }));
                json!({ "ok": true })
            }
            None => {
                self.end_presentation(n >= 0);
                json!({ "ok": true, "fin": true })
            }
        }
    }

    pub fn end_presentation(&self, completed: bool) {
        let Some(p) = self.presentation.borrow_mut().take() else { return };
        let o = self.o.up();
        o.emit(json!({ "type": "presentation_end", "id": p.id, "complete": completed }));
        if self.exploring().as_deref() == Some(p.slug.as_str()) {
            self.close_explore(false);
        }
    }

    /// État de la présentation en cours, pour la card Diction (null si aucune).
    pub fn state(&self) -> J {
        match self.presentation.borrow().as_ref() {
            Some(p) => json!({ "id": p.id, "sujet": js::gs(&self.get(&p.slug).unwrap_or(json!({})), "sujet"), "n": p.current + 1, "total": p.scenes.len(), "pause": p.paused }),
            None => J::Null,
        }
    }

    /// Pilotage de la présentation (voix ou card Diction) : suivant, precedent, debut, pause, reprendre (redit la scène
    /// en cours, avec les corrections de prononciation), stop. false s'il n'y a pas de présentation.
    pub fn control(&self, c: &str) -> bool {
        let Some((id, current)) = self.presentation.borrow().as_ref().map(|p| (p.id.clone(), p.current as i64)) else { return false };
        let o = self.o.up();
        o.emit(json!({ "type": "voice_stop" }));
        match c {
            "suivant" => {
                self.scene(&id, current + 1);
            }
            "precedent" => {
                self.scene(&id, (current - 1).max(0));
            }
            "debut" => {
                self.scene(&id, 0);
            }
            "reprendre" => {
                self.scene(&id, current);
            }
            // Scène « écoute » : le titre montré part dans Spotify ; la présentation se met en pause.
            "ecouter" => {
                let scene = self.presentation.borrow().as_ref().and_then(|p| p.scenes.get(current as usize).cloned()).unwrap_or_default();
                if js::gs(&scene, "type") != "ecoute" {
                    o.emit(json!({ "type": "say", "text": "Il n'y a rien à écouter dans cette scène. Attends la scène d'écoute, ou ouvre l'écran médias à la fin." }));
                    return true;
                }
                if let Some(p) = self.presentation.borrow_mut().as_mut() {
                    p.paused = true;
                }
                o.emit(json!({ "type": "presentation_pause", "id": id }));
                let query = format!("{} {}", js::gs(&scene, "titre"), js::gs(&scene, "artiste")).trim().to_string();
                let w = self.o.clone();
                js::spawn(async move {
                    if let Some(o) = w.upgrade() {
                        let r = o.savoir.listen(&query).await;
                        o.emit(json!({ "type": "say", "text": r["say"] }));
                    }
                });
            }
            "pause" => {
                if let Some(p) = self.presentation.borrow_mut().as_mut() {
                    p.paused = true;
                }
                o.emit(json!({ "type": "presentation_pause", "id": id }));
            }
            _ => {
                self.end_presentation(false);
                o.emit(json!({ "type": "say", "text": "Fin de la présentation." }));
            }
        }
        true
    }

    /// Phrase dite pendant une présentation : pilotage, ou question sur le sujet. true si elle a été prise en charge.
    pub fn on_utterance(&self, folded: &str, raw: &str, routed_locally: bool) -> bool {
        let Some((id, slug)) = self.presentation.borrow().as_ref().map(|p| (p.id.clone(), p.slug.clone())) else { return false };
        let o = self.o.up();
        if let Some(c) = control_of(folded) {
            self.control(c);
            return true;
        }
        if self.exploring().is_some() {
            if let Some(c) = super::explore::control3d_of(folded) {
                self.control3d(c);
                return true;
            }
            if self.show_piece(&slug, folded) {
                return true;
            }
        }
        if routed_locally {
            return false; // une commande connue (musique, volume…) garde son chemin
        }
        let question = raw.trim().to_string();
        let w = self.o.clone();
        o.emit(json!({ "type": "voice_stop" }));
        o.emit(json!({ "type": "presentation_pause", "id": id, "question": question }));
        js::spawn(async move {
            if let Some(o) = w.upgrade() {
                o.savoir.ask(Some(&id), &slug, &question).await;
            }
        });
        true
    }

    /// Question pendant la présentation (`id`) ou explorateur 3D ouvert : réponse tirée du dossier, avec l'image ou la
    /// pièce à montrer.
    pub(super) async fn ask(&self, id: Option<&str>, slug: &str, question: &str) {
        let o = self.o.up();
        let Some(d) = self.get(slug) else { return };
        let hits = self.search(question, Some(slug), 6).await;
        let extraits: Vec<String> = hits.iter().map(|h| js::gs(h, "extrait")).collect();
        if let Some(dm) = self.dossiers.borrow_mut().get_mut(slug) {
            dm["meta"]["questions"] = json!(dm["meta"]["questions"].as_i64().unwrap_or(0) + 1);
        }
        self.save(slug);
        let input = serde_json::to_string(&json!({
            "question": question, "sujet": d["sujet"], "extraits": extraits,
            "images": images(&d).iter().map(|(f, v, l)| json!({ "fichier": f, "vue": v, "legende": l })).collect::<Vec<_>>(),
            "pieces": d["pieces"].as_array().map(|a| a.iter().map(|p| json!({ "id": p["id"], "nom": p["nom"] })).collect::<Vec<_>>())
        }))
        .unwrap_or_default();
        let raw = haiku(&o, ASK_PROMPT, &input, "Question · présentation").await.and_then(|t| super::dossier::parse_loose(&t)).unwrap_or(Value::Null);
        let reponse = Some(txt(&raw["reponse"], 600)).filter(|r| !r.is_empty()).unwrap_or_else(|| {
            extraits.first().map(|e| js::slice_chars(crate::re!(r"^«[^»]*»\s*:\s*").replace(e, "").as_ref(), 300)).unwrap_or_else(|| "Je n'ai rien trouvé à ce sujet dans le dossier.".into())
        });
        let focus = resolve_scene(&json!({ "type": "piece", "piece": raw["piece"], "image": raw["image"], "dit": reponse }), &d)
            .or_else(|| resolve_scene(&json!({ "type": "image", "image": raw["image"], "dit": reponse }), &d))
            .unwrap_or_else(|| json!({ "type": "reponse", "dit": reponse, "image": cover(&d) }));
        let presenting = id.is_some_and(|id| self.presentation.borrow().as_ref().map(|p| p.id == id).unwrap_or(false));
        let exploring = self.exploring().as_deref() == Some(slug);
        if !presenting && !exploring {
            return; // présentation ou explorateur terminés entre-temps
        }
        if presenting {
            o.emit(json!({ "type": "presentation_focus", "id": id, "question": question, "scene": focus }));
        }
        if exploring {
            o.emit(json!({ "type": "savoir_3d_control", "action": "reponse", "question": question, "reponse": reponse, "piece": focus["piece"] }));
        }
        o.emit(json!({ "type": "say", "text": reponse, "presentation": id }));
        o.record("savoir_question", json!({ "dossier": slug, "presentation": true }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier() -> J {
        json!({
            "sujet": "Airbus H160", "categorie": "hélicoptère", "resume": "Hélicoptère bimoteur moyen d'Airbus Helicopters. Il a été certifié en 2020.",
            "fiche": [{ "cle": "Vitesse de croisière", "valeur": "287", "unite": "km/h" }, { "cle": "Moteurs", "valeur": "2 Arrano" }, { "cle": "Passagers", "valeur": "12" }],
            "chronologie": [{ "date": "2015", "fait": "Présentation au salon du Bourget" }, { "date": "2020", "fait": "Certification EASA" }],
            "comparaisons": [{ "autre": "AW169", "points": ["plus léger", "moins bruyant"] }],
            "pieces": [{ "id": "rotor-principal", "nom": "Rotor principal", "description": "Cinq pales Blue Edge.", "position": "haut" }],
            "images": [{ "url": "https://a/x.jpg", "fichier": "0123456789abcdef.jpg", "vue": "profil", "legende": "H160 en vol" },
                       { "url": "https://a/y.jpg", "fichier": "fedcba9876543210.jpg", "vue": "cockpit", "legende": "Cockpit Helionix" },
                       { "url": "https://a/z.jpg", "vue": "detail" }],
            "videos": [{ "url": "https://www.youtube.com/watch?v=abc", "titre": "Présentation" }]
        })
    }

    #[test]
    fn le_script_ne_garde_que_les_donnees_du_dossier() {
        let d = dossier();
        let raw = json!({ "scenes": [
            { "type": "titre", "dit": "Voici le H160 d'Airbus." },
            { "type": "image", "image": "fedcba9876543210.jpg", "dit": "Le cockpit est entièrement numérique." },
            { "type": "image", "image": "inventee.jpg", "dit": "Une image qui n'existe pas." },
            { "type": "chiffres", "cles": ["vitesse de croisière", "Prix inventé"], "dit": "Il file à 287 kilomètres par heure." },
            { "type": "fiche", "cles": ["Moteurs"], "dit": "Une fiche d'une seule ligne." },
            { "type": "comparaison", "autre": "aw169", "dit": "Face à l'AW169, il est plus léger." },
            { "type": "piece", "piece": "Rotor principal", "dit": "Le rotor a cinq pales." },
            { "type": "script", "dit": "Type inconnu, écarté." },
            { "type": "titre", "dit": "Un second titre, écarté." },
            { "type": "conclusion", "dit": "Voilà pour le H160." }
        ] });
        let s = resolve_script(&raw, &d);
        let types: Vec<String> = s.iter().map(|x| js::gs(x, "type")).collect();
        assert_eq!(types, ["titre", "image", "chiffres", "comparaison", "piece", "conclusion"]);
        assert_eq!(s[0]["image"], "0123456789abcdef.jpg", "couverture = vue de profil");
        assert_eq!(s[2]["lignes"].as_array().unwrap().len(), 1);
        assert_eq!(s[2]["lignes"][0]["valeur"], "287");
        assert_eq!(s[3]["points"].as_array().unwrap().len(), 2);
        assert_eq!(s[4]["nom"], "Rotor principal");
    }

    #[test]
    fn un_dossier_en_3d_finit_par_l_explorateur() {
        let raw = json!({ "scenes": [
            { "type": "exploration", "dit": "Place à la maquette en trois dimensions." },
            { "type": "titre", "dit": "Voici le H160 d'Airbus." },
            { "type": "conclusion", "dit": "Voilà pour le H160." }
        ] });
        let types = |s: &[J]| s.iter().map(|x| js::gs(x, "type")).collect::<Vec<_>>();
        assert_eq!(types(&resolve_script(&raw, &dossier())), ["titre", "conclusion"], "sans maquette : pas d'explorateur");
        let mut d = dossier();
        d["maquette"] = json!({ "elements": [{ "forme": "boite" }] });
        let s = resolve_script(&raw, &d);
        assert_eq!(types(&s), ["titre", "conclusion", "exploration"], "toujours en dernier");
        assert_eq!(s[2]["dit"], "Place à la maquette en trois dimensions.", "la narration de l'IA est gardée");
        let s = resolve_script(&json!({ "scenes": [{ "type": "titre", "dit": "Voici le H160 d'Airbus." }] }), &d);
        assert_eq!(types(&s), ["titre", "exploration"], "ajoutée quand l'IA l'oublie");
        assert_eq!(types(&fallback_script(&d)).last().map(String::as_str), Some("exploration"));
    }

    #[test]
    fn presentation_selon_le_profil() {
        let profils = super::super::profils::builtin();
        let profil = |id: &str| profils.iter().find(|p| js::gs(p, "id") == id).unwrap().clone();
        let types = |s: &[J]| s.iter().map(|x| js::gs(x, "type")).collect::<Vec<_>>();
        // Produit : points forts et faibles, prix du moins cher au plus cher, avis.
        let mut d = dossier();
        d["profil_def"] = profil("produit");
        d["points_forts"] = json!(["Silencieux", "Autonomie de 60 minutes"]);
        d["points_faibles"] = json!(["Cher"]);
        d["prix"] = json!([{ "vendeur": "Boulanger", "prix": 749.0, "devise": "€" }, { "vendeur": "Darty", "prix": 699.0, "devise": "€" }]);
        d["avis"] = json!([{ "source": "Les Numériques", "note": "4,5", "sur": "5", "resume": "Excellent aspirateur. Très puissant." }]);
        let s = fallback_script(&d);
        let t = types(&s);
        for want in ["atouts", "prix", "avis"] {
            assert!(t.contains(&want.to_string()), "{want} manque : {t:?}");
        }
        let prix = s.iter().find(|x| js::gs(x, "type") == "prix").unwrap();
        assert_eq!(prix["lignes"][0]["vendeur"], "Darty", "le moins cher d'abord");
        assert!(js::gs(prix, "dit").contains("699 € chez Darty"));
        assert_eq!(t[t.len() - 2..], ["liens".to_string(), "conclusion".to_string()], "sans modèle 3D : les liens remplacent l'explorateur, avant la conclusion");
        // Musique : de quoi écouter, et l'écran médias à la fin.
        let mut m = dossier();
        m["profil_def"] = profil("musique");
        m["ecoute"] = json!([{ "titre": "Discovery", "type": "album", "artiste": "Daft Punk", "annee": "2001" }]);
        let s = fallback_script(&m);
        let t = types(&s);
        assert!(t.contains(&"ecoute".to_string()));
        assert_eq!(t.last().map(String::as_str), Some("media"));
        assert!(!t.contains(&"piece".to_string()), "pas de pièce 3D pour de la musique");
    }

    #[test]
    fn script_de_repli_et_pilotage() {
        let s = fallback_script(&dossier());
        let types: Vec<String> = s.iter().map(|x| js::gs(x, "type")).collect();
        assert_eq!(types.first().map(String::as_str), Some("titre"));
        assert_eq!(types.last().map(String::as_str), Some("conclusion"));
        assert!(types.contains(&"chiffres".to_string()) && types.contains(&"chronologie".to_string()) && types.contains(&"piece".to_string()));
        assert_eq!(control_of("suivant"), Some("suivant"));
        assert_eq!(control_of("reviens en arriere"), Some("precedent"));
        assert_eq!(control_of("arrete la presentation"), Some("stop"));
        assert_eq!(control_of("quelle est sa vitesse"), None);
        assert_eq!(control_of("ecoute le"), Some("ecouter"));
        assert_eq!(control_of("lance l album"), Some("ecouter"));
    }
}
