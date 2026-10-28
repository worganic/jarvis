// Intentions locales (orchestrator/skill-intents.js) : mails, agenda, web, calcul, conversions, météo,
// commandes Windows, applications du menu Démarrer, fichiers et dossiers, projets, rappels,
// programmations, lumières, notes, réunions, listes, achats… Chaque action passe par local_tool() :
// même sécurité (validation par code comprise), même journal et mêmes widgets que pour un modèle.
// `text` est replié et débarrassé de son enrobage ; `raw` garde casse et accents.

use std::future::Future;
use std::path::Path;
use std::rc::Rc;

use serde_json::{json, Value};

use super::intent::Intent;
use super::router::Router;
use super::system::R;
use super::{Fut, OrchRef, Orchestrator, Up};
use crate::js::{self, J};

const MAIL: &str = "(?:e ?mails?|mails?|courriels?|messages? (?:gmail|mail))";

/// « Jean Dupont <jean@x.fr> » → « Jean Dupont » ; « jean@x.fr » → « jean ».
pub fn sender_name(from: &str) -> String {
    let s = from.trim();
    if let Some(m) = crate::re!(r#"^"?([^"<]+?)"?\s*<[^>]+>$"#).captures(s) {
        return m[1].trim().to_string();
    }
    if let Some(m) = crate::re!(r"^<?([^@<\s]+)@").captures(s) {
        return m[1].to_string();
    }
    if s.is_empty() { "un expéditeur inconnu".into() } else { s.to_string() }
}

pub(crate) fn folder_name(k: &str) -> &'static str {
    match k {
        "bureau" => "le bureau",
        "documents" => "tes documents",
        "telechargements" => "tes téléchargements",
        "images" => "tes images",
        "photos" => "tes photos",
        "musique" => "ta musique",
        _ => "tes vidéos",
    }
}

pub(crate) fn ordinal(w: &str) -> Option<i64> {
    Some(match w {
        "premier" | "un" => 1,
        "deuxieme" | "second" | "deux" => 2,
        "troisieme" | "trois" => 3,
        "quatrieme" | "quatre" => 4,
        "cinquieme" | "cinq" => 5,
        "sixieme" | "six" => 6,
        "dernier" => -1,
        _ => return None,
    })
}

pub(crate) fn small_num(w: &str) -> Option<i64> {
    if let Ok(n) = w.parse() {
        return Some(n);
    }
    Some(match w {
        "un" | "une" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "quinze" => 15,
        "vingt" => 20,
        _ => return None,
    })
}

/// Outils de Windows ouverts sans IA (noms repliés).
pub(crate) fn windows_app(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "calculatrice" | "calc" => ("calc", "la calculatrice"),
        "bloc notes" | "notepad" => ("notepad", "le Bloc-notes"),
        "explorateur" | "explorateur de fichiers" | "explorateur windows" | "ce pc" | "poste de travail" => ("explorer", "l'explorateur de fichiers"),
        "gestionnaire des taches" | "gestionnaire de taches" => ("taskmgr", "le gestionnaire des tâches"),
        "panneau de configuration" => ("control", "le panneau de configuration"),
        "invite de commandes" | "invite de commande" => ("cmd", "l'invite de commandes"),
        "terminal" => ("wt", "le terminal"),
        "powershell" => ("powershell", "PowerShell"),
        "paint" => ("mspaint", "Paint"),
        "outil capture" | "outil de capture" => ("snippingtool", "l'outil Capture"),
        "gestionnaire de peripheriques" => ("devmgmt.msc", "le gestionnaire de périphériques"),
        "gestion des disques" => ("diskmgmt.msc", "la gestion des disques"),
        "moniteur de ressources" => ("resmon", "le moniteur de ressources"),
        "observateur d evenements" => ("eventvwr.msc", "l'observateur d'événements"),
        _ => return None,
    })
}

/// Pages des Paramètres de Windows (ms-settings:<page>).
pub(crate) fn settings_page(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "windows" | "systeme" => ("", "les paramètres"),
        "bluetooth" => ("bluetooth", "le Bluetooth"),
        "wifi" | "wi fi" => ("network-wifi", "le Wi-Fi"),
        "reseau" | "internet" => ("network", "le réseau"),
        "mode avion" => ("network-airplanemode", "le mode avion"),
        "son" | "audio" => ("sound", "le son"),
        "affichage" | "ecran" => ("display", "l'affichage"),
        "mode nuit" | "veilleuse" => ("nightlight", "la veilleuse"),
        "windows update" | "mise a jour" | "mises a jour" => ("windowsupdate", "Windows Update"),
        "applications" => ("appsfeatures", "les applications"),
        "applications par defaut" => ("defaultapps", "les applications par défaut"),
        "stockage" => ("storagesense", "le stockage"),
        "batterie" => ("batterysaver", "la batterie"),
        "alimentation" => ("powersleep", "l'alimentation"),
        "fond d ecran" => ("personalization-background", "le fond d'écran"),
        "personnalisation" => ("personalization", "la personnalisation"),
        "couleurs" | "mode sombre" => ("colors", "les couleurs"),
        "notifications" => ("notifications", "les notifications"),
        "imprimantes" | "imprimante" => ("printers", "les imprimantes"),
        "souris" => ("mousetouchpad", "la souris"),
        "pave tactile" => ("devices-touchpad", "le pavé tactile"),
        "clavier" => ("typing", "le clavier"),
        "date et heure" => ("dateandtime", "la date et l'heure"),
        "confidentialite" => ("privacy", "la confidentialité"),
        "vpn" => ("network-vpn", "le VPN"),
        "camera" => ("privacy-webcam", "la caméra"),
        "micro" | "microphone" => ("privacy-microphone", "le micro"),
        "comptes" => ("yourinfo", "les comptes"),
        "economiseur de batterie" => ("batterysaver", "l'économiseur de batterie"),
        _ => return None,
    })
}

pub(crate) fn it<F, Fu>(o: &OrchRef, kind: &str, label: &str, detail: &str, f: F) -> Intent
where
    F: Fn(Rc<Orchestrator>) -> Fu + 'static,
    Fu: Future<Output = J> + 'static,
{
    let o = o.clone();
    let f = Rc::new(f);
    Intent::new(kind, label, detail, move || {
        let (o, f) = (o.clone(), f.clone());
        async move { f(o.up()).await }
    })
}

/// Résultat d'une action système (R) → objet JSON (erreur comprise).
pub fn sysr(r: R) -> J {
    match r {
        Ok(v) => {
            if v.is_null() {
                json!({ "ok": true })
            } else {
                v
            }
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// Action derrière local_tool (sécurité, journal).
pub async fn tool<F, Fu>(o: &Orchestrator, name: &str, params: J, f: F) -> J
where
    F: FnOnce(J) -> Fu + 'static,
    Fu: Future<Output = J> + 'static,
{
    o.local_tool(name, params, Box::new(move |p| -> Fut<J> { Box::pin(f(p)) })).await
}

/// Action système de System.<action> derrière local_tool.
pub async fn sys_tool(o: &Orchestrator, name: &str, action: &'static str, params: J) -> J {
    tool(o, name, params, move |p| async move { super::system::call(action, &p).await.map(sysr).unwrap_or(json!({ "ok": false, "error": "action indisponible" })) }).await
}

pub(crate) fn refused(r: &J) -> Option<J> {
    js::gb(r, "denied").then(|| json!({ "say": "Action refusée par les réglages de sécurité ou annulée.", "display": "Refusé" }))
}

/// Ville par défaut de la météo : dernière demandée, sinon tirée de la mémoire (« habite à Lyon »).
pub fn default_city(o: &Orchestrator) -> Option<String> {
    if let Some(c) = o.status.d.borrow().weather_city.clone().filter(|c| !c.is_empty()) {
        return Some(c);
    }
    for e in o.memory.list() {
        if let Some(m) = crate::re!(r"\b(?:habite|vit|vis|réside|reside|demeure|installée?)\s+(?:à|a|au|en|sur)\s+([A-ZÀ-Ý][\p{L}' -]+?)(?:[.,;]|\s+(?:depuis|avec|et|dans)\b|$)").captures(&js::gs(&e, "text")) {
            return Some(m[1].trim().to_string());
        }
    }
    None
}

/// Ville citée dans la phrase brute : « météo à Lyon », « quel temps fera-t-il demain à Saint-Malo ».
pub fn city_in_raw(raw: &str) -> Option<String> {
    let m = crate::re!(r"(?:^|\s)(?:à|a|au|sur|pour|en)\s+((?:[A-ZÀ-Ý][\p{L}'-]*)(?:[\s-]+(?:[A-ZÀ-Ý][\p{L}'-]*|sur|en|de|la|le|les|d'))*)").captures(raw)?;
    let city = crate::re!(r"(?i)\s+(?:Aujourd'hui|Demain)$").replace(&m[1], "").trim().to_string();
    if crate::re!(r"(?i)^(?:Aujourd|Demain|Lundi|Mardi|Mercredi|Jeudi|Vendredi|Samedi|Dimanche)").is_match(&city) {
        None
    } else {
        Some(city)
    }
}

pub(crate) fn list_fr(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} et {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

pub(crate) fn clean_name(s: &str) -> String {
    crate::re!(r"\s+(?:s il te plait|s il vous plait|stp|svp|merci|maintenant|tout de suite)$").replace_all(s, "").trim().to_string()
}

pub(crate) fn p_plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

// --- Projets ---------------------------------------------------------------------------------------

/// Card numérotée des projets (les numéros valent pour les commandes suivantes).
pub fn project_card(title: &str, list: &[J], hidden_count: usize, empty: &str) -> J {
    use super::projects::ago;
    let rows: Vec<J> = list
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = js::gs(p, "name");
            let fav = js::gb(p, "favorite");
            let hidden = js::gb(p, "hidden");
            let tech = js::str_list(&p["tech"]);
            let sub: Vec<String> = [tech.join(", "), js::gs(p, "description")].into_iter().filter(|s| !s.is_empty()).collect();
            let mut row = json!({
                "icon": if fav { "⭐" } else if hidden { "🗄" } else { "◧" },
                "title": format!("{}. {name}", i + 1), "sub": sub.join(" · "),
                "meta": ago(js::gn_or(p, "lastActivity", 0.0) as i64),
                "say": format!("Où en est le projet {name} ?"),
                "call": { "method": "listAction", "args": ["projet", name, "ouvrir"] },
                "hint": format!("Clic : détails du projet « {name} »"),
                "actions": [
                    { "icon": "▶", "title": format!("Lancer Claude Code dans psmux, dans {name}"), "say": format!("Lance Claude Code dans le projet {name}") },
                    { "icon": ">_", "title": format!("Ouvrir Claude Code dans PowerShell, dans {name}"), "call": { "method": "listAction", "args": ["projet", name, "powershell"] } },
                    if fav { json!({ "icon": "★", "title": "Retirer des favoris", "say": format!("Retire le projet {name} des favoris") }) } else { json!({ "icon": "☆", "title": "Mettre en favori (en tête de liste)", "say": format!("Mets le projet {name} en favori") }) },
                    if hidden { json!({ "icon": "↺", "title": "Remettre dans la liste des projets", "say": format!("Remets le projet {name} dans la liste") }) } else { json!({ "icon": "🗄", "tone": "danger", "title": "Mettre de côté (n’apparaît plus dans la liste)", "say": format!("Mets de côté le projet {name}") }) }
                ]
            });
            if fav {
                row["tone"] = json!("fav");
            }
            row
        })
        .collect();
    let hidden = if hidden_count > 0 { format!(" · {hidden_count} mis de côté : « liste mes projets mis de côté »") } else { String::new() };
    json!({
        "title": title, "tool": "projets", "count": list.len(), "rows": rows, "empty": if empty.is_empty() { "Aucun projet." } else { empty }, "wide": true,
        "footer": format!("▶ Claude Code (psmux) · >_ Claude Code (PowerShell) · ☆ favori · 🗄 mettre de côté · « lance Claude Code dans le projet 2 »{hidden}"),
        "ttl": 180000
    })
}

pub fn project_info_card(info: &J) -> J {
    use super::projects::ago;
    let g = &info["git"];
    let mut rows: Vec<J> = vec![];
    let name = js::gs(info, "name");
    if js::truthy(&info["description"]) {
        rows.push(json!({ "icon": "ℹ", "title": info["description"], "sub": "description" }));
    }
    let tech = js::str_list(&info["tech"]);
    rows.push(json!({ "icon": "⚙", "title": if tech.is_empty() { "non détectées".to_string() } else { tech.join(", ") }, "sub": "technologies" }));
    if g.is_object() {
        let branch = if js::truthy(&g["branch"]) { js::gs(g, "branch") } else { "branche inconnue".into() };
        rows.push(json!({
            "icon": "⎇", "title": branch,
            "sub": if js::truthy(&g["lastCommit"]) { format!("dernier commit : {}", js::gs(g, "lastCommit")) } else { "aucun commit".into() },
            "meta": g["lastCommitAt"].as_i64().filter(|x| *x != 0).map(ago).unwrap_or_default()
        }));
        let ch = js::gn_or(g, "changes", 0.0) as usize;
        rows.push(json!({
            "icon": if ch > 0 { "!" } else { "✓" }, "tone": if ch > 0 { "warn" } else { "ok" },
            "title": if ch > 0 { format!("{ch} fichier{} non commité{}", p_plural(ch), p_plural(ch)) } else { "tout est commité".into() }, "sub": "état git"
        }));
    } else {
        rows.push(json!({ "icon": "⎇", "title": "pas de dépôt git", "sub": "état git" }));
    }
    rows.push(json!({ "icon": "◷", "title": ago(js::gn_or(info, "lastActivity", 0.0) as i64), "sub": "dernière activité" }));
    let subs = js::str_list(&info["subprojects"]);
    if !subs.is_empty() {
        rows.push(json!({ "section": "Sous-projets" }));
        for s in subs {
            rows.push(json!({ "icon": "◧", "title": s, "say": format!("Où en est le projet {s} ?") }));
        }
    }
    rows.push(json!({ "section": "Actions" }));
    rows.push(json!({ "icon": "▶", "title": "Lancer Claude Code ici", "say": format!("Lance Claude Code dans le projet {name}") }));
    rows.push(json!({ "icon": "📂", "title": "Ouvrir le dossier", "say": format!("Ouvre le projet {name}") }));
    let parent = js::gs(info, "parent");
    json!({ "title": if parent.is_empty() { name.clone() } else { format!("{parent} › {name}") }, "tool": "projet", "rows": rows, "footer": info["path"] })
}

// --- Suivi de colis ------------------------------------------------------------------------------------

/// « suis le colis 6A12345678901 de La Poste », « où en est mon colis », « mes colis », « arrête de suivre le colis … ».
pub(crate) fn parcel_intents(o: &OrchRef, t: &str) -> Option<Intent> {
    if let Some(m) = crate::re!(r"^(?:suis|surveille|suivre|ajoute|track)(?: moi)? (?:le |mon |un )?colis (?:numero |n )?([a-z0-9 ]{8,40}?)(?: (?:de |du |chez |avec |par |via )(?:la )?(la poste|colissimo|chronopost|ups|dhl|fedex|amazon|mondial relay|gls|dpd|colis prive|relais colis))?$").captures(t) {
        let (number, carrier) = (m[1].to_string(), m.get(2).map(|c| c.as_str().to_string()).unwrap_or_default());
        return Some(it(o, "parcel_add", "Colis", "ajout d'un colis à suivre", move |o| {
            let (number, carrier) = (number.clone(), carrier.clone());
            async move {
                match o.parcels.add(&number, &carrier) {
                    Ok(p) => {
                        o.parcels.check(&js::gs(&p, "id"), true);
                        json!({ "say": format!("C'est noté, je suis le colis {}. Je regarde où il en est.", js::gs(&p, "numero")), "display": format!("Colis {} suivi", js::gs(&p, "numero")) })
                    }
                    Err(e) => json!({ "say": e, "display": "Colis non ajouté" }),
                }
            }
        }));
    }
    if crate::re!(r"^(?:mes colis|liste (?:de |des |mes )?colis|quels colis (?:je suis|suis je))$").is_match(t) {
        return Some(it(o, "parcel_list", "Colis", "colis suivis", |o| async move {
            let s = o.parcels.summary();
            json!({ "say": s, "display": "Mes colis" })
        }));
    }
    if crate::re!(r"^(?:ou en est|ou est|quel est le statut de|quel est l etat de|des nouvelles de|donne moi des nouvelles de)(?: mon| le| mes| les)? colis?$").is_match(t) {
        return Some(it(o, "parcel_status", "Colis", "état des colis", |o| async move {
            let n = o.parcels.check_all(true);
            let known = o.parcels.summary();
            if n == 0 {
                json!({ "say": if o.parcels.list().is_empty() { "Aucun colis n'est suivi : dis « suis le colis » suivi du numéro.".to_string() } else { known }, "display": "Colis" })
            } else {
                json!({ "say": format!("Je vérifie auprès du transporteur. {known}"), "display": "Colis en cours de vérification" })
            }
        }));
    }
    if let Some(m) = crate::re!(r"^(?:arrete de suivre|oublie|supprime|retire)(?: le| mon)? colis ([a-z0-9 ]+)$").captures(t) {
        let reference = m[1].to_string();
        return Some(it(o, "parcel_remove", "Colis", "colis retiré", move |o| {
            let reference = reference.clone();
            async move {
                match o.parcels.remove(&reference) {
                    Some(p) => json!({ "say": format!("Je ne suis plus le colis {}.", js::gs(&p, "numero")), "display": "Colis retiré" }),
                    None => json!({ "say": "Je ne trouve pas ce colis dans ceux que je suis.", "display": "Colis introuvable" }),
                }
            }
        }));
    }
    None
}

// --- Traduction à la voix ---------------------------------------------------------------------------

/// « traduis où est la gare en espagnol » : traduction par l'IA, dite avec la voix de la langue (translate.rs).
pub(crate) fn translate_intents(o: &OrchRef, t: &str) -> Option<Intent> {
    let (text, key) = super::translate::parse_request(t)?;
    let label = super::translate::LANGUAGES.iter().find(|l| l.0 == key).map(|l| l.2).unwrap_or(key);
    Some(it(o, "translate", "Traduction", &format!("en {label}"), move |o| {
        let text = text.clone();
        async move { o.translate_intent_run(&text, key).await }
    }))
}

// --- Maison connectée (lampes Hue, prises Shelly / Kasa) ----------------------------------------------

/// « allume la cafetière », « éteins la prise du salon », « mets le salon à 40 pour cent » : seulement si le nom dit est celui d'un
/// appareil déclaré (Administration › Maison) ; sinon rien, et les lumières Yeelight prennent la suite.
pub(crate) fn home_intents(o: &OrchRef, t: &str) -> Option<Intent> {
    let names = o.upgrade()?.home.names();
    if names.is_empty() {
        return None;
    }
    let known = |rest: &str| {
        let r = crate::re!(r"[^a-z0-9]+").replace_all(rest, " ").trim().to_string();
        names.iter().any(|n| n.len() >= 3 && (r == *n || r.contains(n.as_str())))
    };
    let mut cmd: Option<J> = None;
    if let Some(m) = crate::re!(r"^(allume|rallume|eteins|eteint|coupe|bascule)(?: moi)? (?:la |le |les |l |ma |mon |mes )?(?:prise |multiprise |lampe |lumiere )?(?:de la |de l |du |des |de )?(.+)$").captures(t) {
        let action = match &m[1] { "allume" | "rallume" => "allumer", "bascule" => "basculer", _ => "eteindre" };
        if known(&m[2]) {
            cmd = Some(json!({ "action": action, "nom": m[2].trim() }));
        }
    } else if let Some(m) = crate::re!(r"^(?:mets |regle )?(?:la |le |l )?(?:lampe |lumiere )?(.+?) (?:a|au) (\d{1,3})(?: ?%| pour ?cent)?$").captures(t) {
        if known(&m[1]) {
            cmd = Some(json!({ "action": "luminosite", "nom": m[1].trim(), "valeur": m[2].parse::<i64>().unwrap_or(50) }));
        }
    }
    let cmd = cmd?;
    let detail = format!("{} · {}", js::gs(&cmd, "action"), js::gs(&cmd, "nom"));
    Some(it(o, "home", "Maison", &detail, move |o| {
        let cmd = cmd.clone();
        async move {
            let oc = o.self_ref();
            let r = tool(&o, "mcp__system__maison", cmd.clone(), move |p| async move { oc.up().home.command(&p).await }).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            if !js::truthy(&r) || r["ok"] == json!(false) {
                return json!({ "say": if js::truthy(&r["message"]) { js::gs(&r, "message") } else { "La maison connectée ne répond pas.".into() }, "display": "Maison indisponible" });
            }
            let nom = js::gs(&cmd, "nom");
            let say = match js::gs(&cmd, "action").as_str() {
                "allumer" => format!("J’allume {nom}."),
                "basculer" => format!("Je bascule {nom}."),
                "luminosite" => format!("{nom} à {} %.", js::s(&cmd["valeur"])),
                _ => format!("J’éteins {nom}."),
            };
            json!({ "say": say, "display": say })
        }
    }))
}

// --- Lumières (Yeelight) ------------------------------------------------------------------------------

const LIGHT: &str = "(?:la |les |ma |mes |l )?(?:lumiere|lumieres|lampe|lampes|ampoule|ampoules)";
const LAMP_OF: &str = "(?: (?:du|de la|de l|des|de) (.+?))?";

pub(crate) fn light_intents(o: &OrchRef, t: &str) -> Option<Intent> {
    let re = |p: String| regex::Regex::new(&p).unwrap();
    let colors = super::lights::COLOR_NAMES.join("|");
    let mut cmd: Option<J> = None;
    if let Some(m) = re(format!("^(?:allume|rallume)(?: moi)? {LIGHT}{LAMP_OF}$")).captures(t) {
        cmd = Some(json!({ "action": "allumer", "lampe": m.get(1).map(|x| x.as_str()) }));
    } else if let Some(m) = re(format!("^(?:eteins|eteint|coupe)(?: moi)? {LIGHT}{LAMP_OF}$")).captures(t) {
        cmd = Some(json!({ "action": "eteindre", "lampe": m.get(1).map(|x| x.as_str()) }));
    } else if let Some(m) = re(format!(r"^(?:mets |regle )?{LIGHT}{LAMP_OF} (?:a|au|sur) (\d{{1,3}})(?: ?%| pour ?cent)?$")).captures(t) {
        cmd = Some(json!({ "action": "luminosite", "lampe": m.get(1).map(|x| x.as_str()), "valeur": m[2].parse::<i64>().unwrap_or(0) }));
    } else if let Some(m) = re(format!("^(?:monte|augmente|remonte)(?: moi)? {LIGHT}{LAMP_OF}$")).captures(t) {
        cmd = Some(json!({ "action": "plus", "lampe": m.get(1).map(|x| x.as_str()) }));
    } else if let Some(m) = re(format!("^(?:baisse|diminue|tamise)(?: moi)? {LIGHT}{LAMP_OF}$")).captures(t) {
        cmd = Some(json!({ "action": "moins", "lampe": m.get(1).map(|x| x.as_str()) }));
    } else if let Some(m) = re(format!("^(?:mets )?{LIGHT}{LAMP_OF} (?:en )?({colors})$")).captures(t) {
        cmd = Some(json!({ "action": "couleur", "lampe": m.get(1).map(|x| x.as_str()), "valeur": &m[2] }));
    } else if let Some(m) = re(format!("^(?:mets )?{LIGHT}{LAMP_OF} (?:plus |en )?(chaude|froide|chaud|froid)$")).captures(t) {
        cmd = Some(json!({ "action": if m[2].starts_with("chaud") { "chaud" } else { "froid" }, "lampe": m.get(1).map(|x| x.as_str()) }));
    } else if let Some(m) = crate::re!(r"^(?:mets |lance |active )?(?:l |une )?ambiance (travail|concentration|detente|cinema|lecture|nuit|soiree)$").captures(t) {
        cmd = Some(json!({ "action": "ambiance", "valeur": &m[1] }));
    } else if crate::re!(r"^(?:quelles (?:sont les |sont mes )?(?:lumieres|lampes|ampoules)|liste (?:les |mes )?(?:lumieres|lampes|ampoules)|(?:detecte|cherche|trouve) (?:les |mes )?(?:lumieres|lampes|ampoules))$").is_match(t) {
        cmd = Some(json!({ "action": "lister" }));
    }
    let mut cmd = cmd?;
    if let Some(l) = cmd["lampe"].as_str() {
        cmd["lampe"] = json!(crate::re!(r"^(?:du|de la|de l|des|de) ").replace(l, "").trim());
    }
    let action = js::gs(&cmd, "action");
    let lampe = js::gs(&cmd, "lampe");
    let val = if cmd["valeur"].is_null() { String::new() } else { format!(" {}", js::s(&cmd["valeur"])) };
    let detail = format!("{action}{val}{}", if lampe.is_empty() { String::new() } else { format!(" · {lampe}") });
    Some(it(o, "lights", "Lumières", &detail, move |o| {
        let cmd = cmd.clone();
        async move {
            let action = js::gs(&cmd, "action");
            if action == "lister" {
                o.lights.discover().await;
            }
            let oc = o.self_ref();
            let r = tool(&o, "mcp__system__lumiere", cmd.clone(), move |p| async move { oc.up().lights.command(&p).await }).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            if !js::truthy(&r) || r["ok"] == json!(false) {
                return json!({ "say": if js::truthy(&r["message"]) { js::gs(&r, "message") } else { "Les lumières ne répondent pas.".into() }, "display": "Lumières indisponibles" });
            }
            if action == "lister" {
                let lampes = js::arr(&r["lampes"]);
                let rows: Vec<J> = lampes
                    .iter()
                    .map(|l| {
                        let on = js::gb(l, "allumee");
                        let mut row = json!({ "icon": "💡", "title": l["nom"], "sub": l["ip"], "meta": if on { "allumée" } else { "éteinte" }, "say": format!("{} la lumière {}", if on { "Éteins" } else { "Allume" }, js::gs(l, "nom")) });
                        if on {
                            row["tone"] = json!("ok");
                        }
                        row
                    })
                    .collect();
                o.show_widget("tool", json!({ "title": "Mes lumières", "tool": "yeelight", "count": lampes.len(), "rows": rows }));
                let names: Vec<String> = lampes.iter().map(|l| js::gs(l, "nom")).collect();
                return json!({ "say": format!("{} lampe{} : {}.", lampes.len(), p_plural(lampes.len()), names.join(", ")), "display": format!("{} lampes", lampes.len()) });
            }
            let lampe = js::gs(&cmd, "lampe");
            let w = if lampe.is_empty() { String::new() } else { format!(" {lampe}") };
            let v = js::s(&cmd["valeur"]);
            let say = match action.as_str() {
                "luminosite" => format!("Lumière{w} à {v} %."),
                "couleur" => format!("Lumière{w} en {v}."),
                "chaud" => format!("Lumière{w} chaude."),
                "froid" => format!("Lumière{w} froide."),
                "ambiance" => format!("Ambiance {}.", super::lights::SCENES.iter().find(|s| s.0 == v).map(|s| s.1).unwrap_or(&v)),
                a => {
                    let verb = match a {
                        "allumer" => "J’allume",
                        "eteindre" => "J’éteins",
                        "plus" => "Je monte",
                        "moins" => "Je baisse",
                        _ => "C’est fait",
                    };
                    format!("{verb} la lumière{w}.")
                }
            };
            json!({ "say": say, "display": say })
        }
    }))
}

// --- Rappels et programmations ---------------------------------------------------------------------

fn schedule_card(o: &Orchestrator) -> Vec<J> {
    use super::schedules::{action_text, is_sensitive};
    let s = &o.schedules;
    let list = s.upcoming();
    let mut rows = vec![];
    if let Some(active) = s.active_countdown() {
        rows.push(json!({ "icon": "⏳", "tone": "warn", "title": format!("{} : compte à rebours en cours", action_text(&active["action"])), "sub": "dis « annule », Échap ou clique ici pour annuler", "say": "Annule le compte à rebours" }));
    }
    for i in &list {
        let sens = is_sensitive(&i["action"]);
        let mut row = json!({
            "icon": if js::gs(&i["trigger"], "type") == "claude_done" { "◈" } else { "⏰" },
            "title": action_text(&i["action"]),
            "sub": format!("{}{}", s.when_text(i), if sens { " · 120 s de compte à rebours avant" } else { "" }),
            "meta": format!("n° {}", js::s(&i["id"])),
            "say": format!("Annule la programmation {}", js::s(&i["id"])),
            "hint": "cliquer pour annuler cette programmation"
        });
        if sens {
            row["tone"] = json!("warn");
        }
        rows.push(row);
    }
    let n = rows.len();
    o.show_widget("tool", json!({ "title": "Programmations", "tool": "programmations", "count": n, "ttl": 120000, "rows": rows, "empty": "Aucune programmation.", "footer": "clique sur une programmation pour l’annuler · Administration › Programmations" }));
    list
}

pub(crate) fn schedule_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    use super::schedules as S;
    if crate::re!(r"^(?:quelles sont |liste |montre moi |affiche |donne moi )?(?:mes |les )?(?:programmations?|actions programmees|taches programmees)(?: en cours| prevues| a venir)?$").is_match(t) {
        return Some(it(o, "schedules_list", "Programmations", "liste", |o| async move {
            let list = schedule_card(&o);
            if list.is_empty() && o.schedules.active_countdown().is_none() {
                return json!({ "say": "Tu n'as aucune programmation.", "display": "Aucune programmation" });
            }
            let next = list.first().map(|f| {
                let when = o.schedules.when_text(f);
                format!(". La prochaine : {}, {}", S::action_text(&f["action"]).replace(['«', '»'], ""), when.split(" · ").next().unwrap_or(""))
            });
            json!({ "say": format!("Tu as {} programmation{}{}.", list.len(), p_plural(list.len()), next.unwrap_or_default()), "display": format!("{} programmations", list.len()) })
        }));
    }
    if crate::re!(r"^(?:annule|arrete|stoppe)(?: le| l)? (?:compte a rebours|decompte|chrono|extinction|redemarrage|mise en veille|hibernation|verrouillage)$").is_match(t) {
        return Some(it(o, "schedule_countdown_cancel", "Programmation", "compte à rebours", |o| async move {
            match o.schedules.cancel_countdown("voix") {
                Some(_) => json!({ "say": "Annulé. Rien ne sera fait.", "display": "Compte à rebours annulé" }),
                None => json!({ "say": "Il n'y a aucun compte à rebours en cours.", "display": "Rien à annuler" }),
            }
        }));
    }
    let all = crate::re!(r"^(?:annule|supprime|efface|retire) (?:toutes |tous )?(?:mes |les )?programmations$").is_match(t);
    let one = crate::re!(r"^(?:annule|supprime|efface|retire)(?: la| ma)? programmation(?: numero| n)? (\d+|un|deux|trois|quatre|cinq|six|sept|huit|neuf|dix)$").captures(t).map(|m| m[1].to_string());
    if all || one.is_some() {
        let detail = if all { "toutes".to_string() } else { one.clone().unwrap() };
        return Some(it(o, "schedules_cancel", "Programmations", &detail, move |o| {
            let one = one.clone();
            async move {
                if all {
                    let n = o.schedules.cancel_all();
                    return json!({ "say": if n > 0 { format!("{n} programmation{} annulée{}.", p_plural(n), p_plural(n)) } else { "Tu n'avais aucune programmation.".into() }, "display": "Programmations annulées" });
                }
                let id = one.and_then(|w| small_num(&w)).unwrap_or(0);
                if !o.schedules.cancel(id, "voix") {
                    return json!({ "say": "Je ne trouve pas cette programmation. Dis « quelles sont mes programmations » pour les voir.", "display": "Programmation introuvable" });
                }
                json!({ "say": format!("Programmation {id} annulée."), "display": "Programmation annulée" })
            }
        }));
    }
    let parsed = S::parse_schedule(t, raw)?;
    let detail = if parsed["action"].is_object() { S::action_text(&parsed["action"]) } else { "erreur".into() };
    Some(it(o, "schedule_create", "Programmation", &detail, move |o| {
        let parsed = parsed.clone();
        async move {
            if js::truthy(&parsed["error"]) {
                return json!({ "say": parsed["error"], "display": parsed["error"] });
            }
            let r = o.schedules.add(&parsed["trigger"], &parsed["action"]);
            if !js::gb(&r, "ok") {
                return json!({ "say": r["error"], "display": r["error"] });
            }
            let item = r["item"].clone();
            let id = item["id"].as_i64().unwrap_or(0);
            let text = S::action_text(&item["action"]);
            let oc = o.self_ref();
            o.undo.add(&format!("programmation « {text} »"), Box::new(move || Box::pin(async move { if oc.up().schedules.cancel(id, "annuler") { "Programmation supprimée.".to_string() } else { "Cette programmation n’existe plus.".to_string() } })));
            schedule_card(&o);
            let when = if js::gs(&item["trigger"], "type") == "claude_done" {
                "quand Claude aura terminé".to_string()
            } else {
                let due = item["trigger"]["dueAt"].as_i64().unwrap_or(0);
                format!("{}, dans {}", S::when_label(due), S::remaining_label(due - js::now_ms()))
            };
            let guard = if S::is_sensitive(&item["action"]) { format!(" Avant d'agir, je lancerai un compte à rebours de {} minutes dans le cœur : dis « annule » pour l'arrêter.", S::COUNTDOWN_S / 60) } else { String::new() };
            json!({ "say": format!("C'est programmé : {} {when}.{guard}", text.replace(['«', '»'], "").to_lowercase()), "display": format!("{text} · {when}") })
        }
    }))
}

pub(crate) fn reminder_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    use super::reminders::{parse_reminder, when_label};
    if crate::re!(r"^(?:quels sont |liste |montre moi |affiche |donne moi |c est quoi )?(?:mes|les) rappels(?: en cours| prevus| a venir| programmes)?$").is_match(t) || crate::re!(r"^(?:qu est ce que j ai|j ai quoi) comme rappels$").is_match(t) {
        return Some(it(o, "reminders_list", "Rappels", "liste", |o| async move {
            let list = o.reminders.upcoming();
            o.list_cards.reminders_card();
            let Some(first) = list.first() else { return json!({ "say": "Tu n'as aucun rappel programmé.", "display": "Aucun rappel" }) };
            let what = if js::truthy(&first["what"]) { js::gs(first, "what") } else { "un rappel".into() };
            json!({ "say": format!("Tu as {} rappel{}. Le prochain : {what}, {}.", list.len(), p_plural(list.len()), when_label(first["at"].as_i64().unwrap_or(0))), "display": format!("{} rappels", list.len()) })
        }));
    }
    let all = crate::re!(r"^(?:annule|supprime|efface|retire) (?:tous )?(?:mes|les) rappels$").is_match(t);
    let one = crate::re!(r"^(?:annule|supprime|efface|retire)(?: le| mon)? rappel(?: numero| n)? (\d+|un|deux|trois|quatre|cinq|six|sept|huit|neuf|dix)$").captures(t).or_else(|| crate::re!(r"^(?:annule|supprime|efface|retire)(?: le| mon)? rappel (?:de |d |pour )(.+)$").captures(t)).map(|m| m[1].to_string());
    if all || one.is_some() {
        let detail = if all { "tous".to_string() } else { one.clone().unwrap() };
        return Some(it(o, "reminders_cancel", "Rappels", &detail, move |o| {
            let key = one.clone();
            async move {
                if all {
                    let n = o.reminders.cancel_all();
                    return json!({ "say": if n > 0 { format!("{n} rappel{} supprimé{}.", p_plural(n), p_plural(n)) } else { "Tu n'avais aucun rappel.".into() }, "display": "Rappels supprimés" });
                }
                let key = key.unwrap_or_default();
                let upcoming = o.reminders.upcoming();
                let target = match small_num(&key).filter(|_| crate::re!(r"^(?:\d+|un|deux|trois|quatre|cinq|six|sept|huit|neuf|dix)$").is_match(&key)) {
                    Some(n) => {
                        let shown = o.list_cards.by_number("rappel", n);
                        shown.and_then(|id| upcoming.iter().find(|r| r["id"] == id).cloned()).or_else(|| n.checked_sub(1).and_then(|i| upcoming.get(i as usize).cloned()))
                    }
                    None => {
                        let k = super::projects::fold_name(&key);
                        upcoming.iter().find(|r| super::projects::fold_name(&js::gs(r, "what")).contains(&k)).cloned()
                    }
                };
                let ok = target.as_ref().map(|t| o.reminders.cancel(t["id"].as_i64().unwrap_or(0))).unwrap_or(false);
                if !ok {
                    return json!({ "say": "Je ne trouve pas ce rappel. Dis « quels sont mes rappels » pour les voir.", "display": "Rappel introuvable" });
                }
                let t = target.unwrap();
                json!({ "say": format!("Rappel annulé : {}.", if js::truthy(&t["what"]) { js::gs(&t, "what") } else { "rappel".into() }), "display": "Rappel annulé" })
            }
        }));
    }
    if let Some(m) = crate::re!(r"^(?:reporte|decale|repousse)(?: le| mon)? rappel(?: numero| n)? (\d+|un|deux|trois|quatre|cinq|six|sept|huit|neuf|dix)(?: d une heure)?$").captures(t) {
        let n = small_num(&m[1]).unwrap_or(0);
        return Some(it(o, "reminders_postpone", "Rappels", &format!("n° {n}"), move |o| async move {
            let id = o.list_cards.by_number("rappel", n).or_else(|| n.checked_sub(1).and_then(|i| o.reminders.upcoming().get(i as usize).map(|r| r["id"].clone()))).unwrap_or(Value::Null);
            let r = o.list_cards.action("rappel", &id, "reporter").await;
            json!({ "say": r["say"], "display": if js::gb(&r, "ok") { json!("Rappel reporté") } else { r["say"].clone() } })
        }));
    }
    let parsed = parse_reminder(t, raw)?;
    let what = js::gs(&parsed, "what");
    Some(it(o, "reminder_create", "Rappel", if what.is_empty() { "rappel" } else { &what }, move |o| {
        let parsed = parsed.clone();
        async move {
            let what = js::gs(&parsed, "what");
            if js::gb(&parsed, "missingWhen") {
                let de = if what.is_empty() { "de…".to_string() } else if crate::re!(r"(?i)^[aeiouyéèêàâîôûh]").is_match(&what) { format!("d'{what}") } else { format!("de {what}") };
                return json!({ "say": format!("Pour quand ? Dis par exemple : « rappelle-moi demain à 9 heures {de} »."), "display": "Heure manquante" });
            }
            let r = o.reminders.add(&what, parsed["at"].as_i64().unwrap_or(0), parsed["repeat"].clone());
            let id = r["id"].as_i64().unwrap_or(0);
            let label = if what.is_empty() { "rappel".to_string() } else { what.clone() };
            let oc = o.self_ref();
            let l2 = label.clone();
            o.undo.add(&format!("rappel « {label} »"), Box::new(move || Box::pin(async move { if oc.up().reminders.cancel(id) { format!("Rappel « {l2} » supprimé.") } else { "Ce rappel n’existe plus.".to_string() } })));
            let every = if r["repeat"].is_object() { if js::gs(&r["repeat"], "every") == "week" { ", chaque semaine" } else { ", chaque jour" } } else { "" };
            let when = when_label(r["at"].as_i64().unwrap_or(0));
            json!({ "say": format!("C'est noté : je te le rappelle {when}{every}."), "display": format!("{} · {when}", if what.is_empty() { "Rappel".to_string() } else { what }) })
        }
    }))
}

// La suite (projets, mails, agenda, Windows, fichiers…) est dans skill_intents_more.rs.
pub use super::skill_intents_more::skill_intent;

#[allow(unused)]
fn _p(_: &Path, _: &Router) {}
