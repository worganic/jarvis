// Suite des intentions locales (orchestrator/skill-intents.js) : projets, exclusions de mails, macros,
// automatisations, notes, réunions, listes, dessin, fichiers, cards, bureau, briefing, compétences,
// lumières, programmations, rappels, mails, agenda, météo, calcul, commandes Windows, documents,
// webcam, recherche, achats, web et applications du menu Démarrer — dans l'ordre de la v3.

use std::path::Path;

use serde_json::{json, Value};

use super::intent::Intent;
use super::local_skills as skills;
use super::router::Router;
use super::skill_intents::{
    clean_name, folder_name, home_intents, it, light_intents, parcel_intents, translate_intents, list_fr, ordinal, p_plural, refused, reminder_intents, schedule_intents, sender_name, settings_page, small_num, sys_tool, sysr, tool, windows_app,
};
use super::{OrchRef, Orchestrator, Up};
use crate::js::{self, J};

const MAIL: &str = "(?:e ?mails?|mails?|courriels?|messages? (?:gmail|mail))";

macro_rules! lazy_re {
    ($name:ident, $build:expr) => {
        fn $name() -> &'static regex::Regex {
            static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            R.get_or_init(|| regex::Regex::new(&$build).unwrap())
        }
    };
}

fn basename(p: &str) -> String {
    Path::new(p).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string())
}

// --- Projets -----------------------------------------------------------------------------------------

/// Projet désigné par son nom, ou par « ici » / « ce projet » (déduit du contexte).
#[derive(Clone)]
struct Target {
    spoken: String,
    here: bool,
    found: Option<J>,
}

impl Target {
    fn new(o: &Orchestrator, spoken: &str) -> Self {
        let here = super::context::is_here(spoken);
        Self { spoken: spoken.to_string(), here, found: if here { None } else { o.projects.find(spoken) } }
    }

    async fn get(&self, o: &Orchestrator) -> Option<J> {
        let p = if self.here { o.context.current_project().await } else { self.found.clone() };
        if let Some(p) = &p {
            o.context.remember(p);
        }
        p
    }

    fn missing(&self, o: &Orchestrator) -> J {
        if self.here {
            return json!({ "say": "Je ne vois pas sur quel projet tu travailles. Ouvre-le dans VS Code ou dans l'explorateur, ou dis son nom.", "display": "Projet courant inconnu" });
        }
        let names: Vec<String> = o.projects.list().iter().filter(|p| !js::truthy(&p["parent"])).map(|p| js::gs(p, "name")).take(8).collect();
        json!({ "say": format!("Je ne trouve pas de projet {}. Tes projets sont {}.", self.spoken, list_fr(&names)), "display": "Projet introuvable" })
    }

    fn label(&self) -> String {
        self.found.as_ref().map(|f| js::gs(f, "name")).unwrap_or_else(|| self.spoken.clone())
    }
}

fn hidden_count(o: &Orchestrator) -> usize {
    o.projects.list().iter().filter(|p| !js::truthy(&p["parent"]) && o.projects.is_hidden(p)).count()
}

fn project_intents(o: &OrchRef, t: &str) -> Option<Intent> {
    let orch = o.up();
    let root = orch.projects.root();
    let show_list = |scope: &'static str, title: &'static str| {
        it(o, "projects_list", title, &root, move |o| async move {
            let all = scope != "liste";
            let oc = o.self_ref();
            let params = if all { json!({ "action": "lister", "tous": true }) } else { json!({ "action": "lister" }) };
            let r = tool(&o, "mcp__system__projets", params, move |_| async move { oc.up().projects.tool(&json!({ "action": "lister", "tous": all })).await }).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            if !js::truthy(&r) || r["ok"] == json!(false) {
                return json!({ "say": format!("Je ne trouve pas le dossier de tes projets : {}. Tu peux le changer dans l'administration, section Projets.", o.projects.root()), "display": "Dossier introuvable" });
            }
            let list = o.projects.ordered(scope).await;
            let hidden = if scope == "liste" { hidden_count(&o) } else { 0 };
            o.projects.remember_numbers(&list);
            let empty = match scope {
                "favoris" => "Aucun favori : ☆ sur un projet pour le mettre en avant.",
                "de_cote" => "Aucun projet mis de côté.",
                _ => "",
            };
            o.show_widget("tool", super::skill_intents::project_card(title, &list, hidden, empty));
            if list.is_empty() {
                let say = match scope {
                    "favoris" => "Tu n'as pas encore de projet favori.",
                    "de_cote" => "Aucun projet n'est mis de côté.",
                    _ => "Je ne trouve aucun projet.",
                };
                return json!({ "say": say, "display": "0 projet" });
            }
            let names = |l: &[J], n: usize| -> Vec<String> { l.iter().take(n).map(|p| js::gs(p, "name")).collect() };
            let fav: Vec<J> = list.iter().filter(|p| js::gb(p, "favorite")).cloned().collect();
            let n = list.len();
            let lead = match scope {
                "de_cote" => format!("{n} projet{} mis de côté", p_plural(n)),
                "favoris" => format!("{n} favori{} : {}", p_plural(n), list_fr(&names(&list, 5))),
                _ => format!(
                    "Tu as {n} projets{}",
                    if !fav.is_empty() { format!(", dont {} favori{} : {}", fav.len(), p_plural(fav.len()), list_fr(&names(&fav, 3))) } else { format!(". Les plus récents : {}", list_fr(&names(&list, 3))) }
                ),
            };
            json!({ "say": format!("{lead}. La liste numérotée est à l'écran."), "display": format!("{n} projets") })
        })
    };
    if crate::re!(r"^(?:(?:liste|montre|affiche|donne|quels sont)(?: moi)? (?:mes |les )?projets (?:mis de cote|de cote|masques|caches|archives|ignores|exclus)|(?:mes )?projets (?:mis de cote|masques|archives))$").is_match(t) {
        return Some(show_list("de_cote", "Projets mis de côté"));
    }
    if crate::re!(r"^(?:(?:liste|montre|affiche|donne|quels sont)(?: moi)? (?:mes |les )?projets (?:favoris|preferes|en favori)|(?:mes )?projets favoris|mes favoris)$").is_match(t) {
        return Some(show_list("favoris", "Projets favoris"));
    }
    if crate::re!(r"^(?:(?:liste|montre|affiche|donne|quels sont)(?: moi)? (?:la liste de )?)?tous (?:mes|les) projets(?: meme (?:ceux )?mis de cote| y compris (?:ceux )?mis de cote)?$").is_match(t) {
        return Some(show_list("tous", "Tous mes projets"));
    }
    if crate::re!(r"^(?:quels? sont|quelle est la liste de|liste|donne moi|montre moi|affiche|dis moi|c est quoi|rappelle moi)(?: moi)?(?: la liste)?(?: de| des)? (?:mes|les) projets(?: en cours)?$").is_match(t)
        || crate::re!(r"^(?:mes projets|liste (?:de mes|des) projets|combien (?:de projets|j ai de projets|ai je de projets)(?: en cours)?)$").is_match(t)
    {
        return Some(show_list("liste", "Mes projets"));
    }
    if crate::re!(r"\b(?:sur quoi (?:j ai|ai je) (?:travaille|bosse)(?: recemment| dernierement| en dernier| cette semaine)?|mon dernier projet|mes derniers projets|(?:mes )?projets? (?:les plus )?recents?)\b").is_match(t) {
        return Some(it(o, "projects_recent", "Projets récents", &root, |o| async move {
            let oc = o.self_ref();
            let r = tool(&o, "mcp__system__projets", json!({ "action": "lister" }), move |_| async move { oc.up().projects.tool(&json!({ "action": "lister" })).await }).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            if !js::truthy(&r) || r["ok"] == json!(false) || js::arr(&r["projets"]).is_empty() {
                return json!({ "say": "Je ne trouve aucun projet.", "display": "Aucun projet" });
            }
            let mut recent = o.projects.ordered("liste").await;
            recent.sort_by(|a, b| js::gn_or(b, "lastActivity", 0.0).partial_cmp(&js::gn_or(a, "lastActivity", 0.0)).unwrap_or(std::cmp::Ordering::Equal));
            recent.truncate(6);
            o.projects.remember_numbers(&recent);
            o.show_widget("tool", super::skill_intents::project_card("Projets récents", &recent, 0, ""));
            let said: Vec<String> = recent.iter().take(3).map(|p| format!("{}, {}", js::gs(p, "name"), super::projects::ago(js::gn_or(p, "lastActivity", 0.0) as i64))).collect();
            json!({
                "say": format!("Ton projet le plus récent est {}. Ensuite {}.", said.first().cloned().unwrap_or_default(), list_fr(&said[1.min(said.len())..])),
                "display": recent.iter().take(3).map(|p| js::gs(p, "name")).collect::<Vec<_>>().join(" · ")
            })
        }));
    }

    // Favoris et projets mis de côté.
    const P: &str = "(?:le |mon |ma |la )?projet (?:numero |n )?";
    let re = |p: String| regex::Regex::new(&p).unwrap();
    let fav_add = re(format!("^(?:mets|met|ajoute|place|passe|marque)(?: moi)? {P}(.+?) (?:en|dans les|aux|dans mes|comme) favoris?$")).captures(t)
        .or_else(|| re(format!("^(?:mets|ajoute)(?: moi)? (?:en|aux|dans les|dans mes) favoris? {P}(.+)$")).captures(t))
        .or_else(|| re(format!("^(?:mets en avant|epingle|favorise) {P}(.+)$")).captures(t));
    let fav_del = re(format!("^(?:retire|enleve|supprime|sors|ote)(?: moi)? {P}(.+?) (?:des|de mes) favoris$")).captures(t).or_else(|| re(format!("^(?:retire|enleve|sors) (?:des|de mes) favoris {P}(.+)$")).captures(t));
    let hide = re(format!("^(?:mets|met|range|laisse|place)(?: moi)? de cote {P}(.+)$")).captures(t)
        .or_else(|| re(format!("^(?:mets|met|range|laisse|place)(?: moi)? {P}(.+?) de cote$")).captures(t))
        .or_else(|| re(format!("^(?:masque|cache|archive|ignore|retire de la liste) {P}(.+)$")).captures(t));
    let unhide = re(format!("^(?:remets|ressors|sors|reprends|reactive|demasque|desarchive|reaffiche)(?: moi)? {P}(.+?)(?: dans (?:la liste|mes projets|la liste des projets)| de cote)?$")).captures(t);
    let kind = if fav_add.is_some() { "project_favorite" } else if fav_del.is_some() { "project_unfavorite" } else if hide.is_some() { "project_hide" } else { "project_unhide" };
    if let Some(m) = fav_add.or(fav_del).or(hide).or(unhide) {
        let spoken = clean_name(&m[1]);
        let found = orch.projects.find(&spoken);
        let detail = found.as_ref().map(|f| js::gs(f, "name")).unwrap_or_else(|| spoken.clone());
        return Some(it(o, kind, "Projets", &detail, move |o| {
            let (spoken, found) = (spoken.clone(), found.clone());
            async move {
                let Some(found) = found else { return json!({ "say": format!("Je ne trouve pas de projet {spoken}. Dis « liste mes projets » pour voir les numéros."), "display": "Projet introuvable" }) };
                let changed = match kind {
                    "project_favorite" => o.projects.set_favorite(&found, true),
                    "project_unfavorite" => o.projects.set_favorite(&found, false),
                    "project_hide" => o.projects.set_hidden(&found, true),
                    _ => o.projects.set_hidden(&found, false),
                };
                let name = js::gs(&found, "name");
                o.record("projet_preference", json!({ "projet": name, "action": kind.trim_start_matches("project_") }));
                let scope = if kind == "project_unhide" { "de_cote" } else { "liste" };
                let list = o.projects.ordered(scope).await;
                o.projects.remember_numbers(&list);
                let hidden = if scope == "liste" { hidden_count(&o) } else { 0 };
                o.show_widget("tool", super::skill_intents::project_card(if scope == "de_cote" { "Projets mis de côté" } else { "Mes projets" }, &list, hidden, if scope == "de_cote" { "Aucun projet mis de côté." } else { "" }));
                let say = match (kind, changed) {
                    ("project_favorite", true) => format!("{name} est en favori : il passe en tête de liste."),
                    ("project_favorite", false) => format!("{name} est déjà en favori."),
                    ("project_unfavorite", true) => format!("{name} n'est plus en favori."),
                    ("project_unfavorite", false) => format!("{name} n'était pas en favori."),
                    ("project_hide", true) => format!("{name} est mis de côté : il n'apparaîtra plus dans la liste de tes projets."),
                    ("project_hide", false) => format!("{name} était déjà mis de côté."),
                    (_, true) => format!("{name} est de retour dans la liste de tes projets."),
                    _ => format!("{name} n'était pas mis de côté."),
                };
                json!({ "say": say, "display": name })
            }
        }));
    }

    // Lancer Claude Code dans un projet (sans avoir à dire « tmux »).
    let claude = crate::re!(r"^(?:lance|ouvre|demarre|mets|relance)(?: moi)?(?: une session)?(?: de)? claude(?: code)? (?:(?:dans|sur|pour|avec) (?:le |la |mon |ma )?(?:projet |dossier )?(.+)|(ici|la dedans))$").captures(t)
        .or_else(|| crate::re!(r"^(?:travaille|bosse|code|developpe) (?:sur|dans) (?:le |mon )?projet (.+?) avec claude(?: code)?$").captures(t))
        .or_else(|| crate::re!(r"^(?:(?:lance|ouvre|demarre)(?: moi)? )?(?:un |une )?(?:psmux|session psmux)(?: avec claude(?: code)?)? (?:dans|sur|pour) (?:le |la |mon |ma )?(?:projet |dossier )?(.+)$").captures(t));
    if let Some(m) = claude {
        let spoken = clean_name(m.get(1).or(m.get(2)).map(|x| x.as_str()).unwrap_or(""));
        let spoken = crate::re!(r" (?:dans|sur) tmux$").replace(&spoken, "").into_owned();
        let dest = Target::new(&orch, &spoken);
        return Some(it(o, "projects_claude", "Claude Code", &dest.label(), move |o| {
            let dest = dest.clone();
            async move {
                let Some(project) = dest.get(&o).await else { return dest.missing(&o) };
                let (path, name) = (js::gs(&project, "path"), js::gs(&project, "name"));
                let (r, _, command) = o.tmux_voice.launch_claude(Some(&path)).await;
                if js::gb(&r, "ok") {
                    return json!({ "say": format!("Claude Code est lancé dans {name}. Dis « parle à Claude » pour que je lui transmette ce que tu dis."), "display": format!("{name} · tmux") });
                }
                if js::gb(&r, "denied") {
                    return json!({ "say": "Lancement refusé.", "display": "Refusé" });
                }
                // psmux absent ou en panne : fenêtre de terminal ordinaire dans le dossier du projet.
                super::projects::open_claude_window(&path, &name, &command);
                o.record("action_locale", json!({ "outil": "claude_terminal", "detail": path }));
                json!({ "say": format!("Claude Code s'ouvre dans {name}, dans une fenêtre de terminal."), "display": format!("{name} · terminal") })
            }
        }));
    }

    // Ouvrir le dossier d'un projet (explorateur ou VS Code).
    let open = crate::re!(r"^(?:ouvre|affiche|montre)(?: moi)? (?:le |la )?(?:dossier (?:du |de mon |de )?)?(?:mon |le )?projet (.+?)(?: (?:dans|avec|sous) (vs code|vscode|visual studio code|code|l explorateur|explorateur))?$").captures(t)
        .or_else(|| crate::re!(r"^(?:ouvre|affiche|montre)(?: moi)? (ce projet|ce dossier)(?: (?:dans|avec|sous) (vs code|vscode|visual studio code|code|l explorateur|explorateur))?$").captures(t));
    if let Some(m) = open {
        let spoken = clean_name(&m[1]);
        let in_code = m.get(2).map(|x| !x.as_str().contains("explorateur")).unwrap_or(false);
        let dest = Target::new(&orch, &spoken);
        return Some(it(o, "projects_open", "Ouvrir un projet", &dest.label(), move |o| {
            let dest = dest.clone();
            async move {
                let Some(project) = dest.get(&o).await else { return dest.missing(&o) };
                let (path, name) = (js::gs(&project, "path"), js::gs(&project, "name"));
                if in_code {
                    let p2 = path.clone();
                    let r = tool(&o, "mcp__system__ouvrir_application", json!({ "application": "VS Code", "dossier": path }), move |_| async move {
                        crate::proc::detached("cmd.exe", &["/c", "code", &p2]);
                        json!({ "ok": true })
                    })
                    .await;
                    if let Some(x) = refused(&r) {
                        return x;
                    }
                    return json!({ "say": format!("J'ouvre {name} dans VS Code."), "display": format!("{name} · VS Code") });
                }
                let r = sys_tool(&o, "mcp__system__ouvrir_dossier", "ouvrir_dossier", json!({ "chemin": path })).await;
                if let Some(x) = refused(&r) {
                    return x;
                }
                json!({ "say": format!("J'ouvre le dossier {name}."), "display": name })
            }
        }));
    }

    // Parler d'un projet : « parle-moi du projet stats-ia », « où en est jarvis v2 ».
    let about = crate::re!(r"^(?:parle moi|dis moi tout|dis m en plus|raconte moi|c est quoi|qu est ce que c est|infos?|informations|resume|fais (?:moi )?le point)(?: sur| du| de| des| a propos du?)? (ce projet|ce dossier)$").captures(t)
        .or_else(|| crate::re!(r"^(?:parle moi|dis moi tout|dis m en plus|raconte moi|c est quoi|qu est ce que c est|infos?|informations|resume|fais (?:moi )?le point)(?: sur| du| de| des| a propos du?)? (?:le |mon )?projet (.+)$").captures(t))
        .or_else(|| crate::re!(r"^(?:ou en est|ou en suis je (?:avec|sur|dans)|ou j en suis (?:avec|sur|dans)|quel est l etat (?:de|du)|etat du|fais le point sur) (?:le |mon )?(projet .+|.+)$").captures(t));
    if let Some(m) = about {
        let spoken = crate::re!(r"^projet (courant|actuel|ouvert|en cours)").replace(&clean_name(&m[1]), "le projet $1").into_owned();
        let explicit = t.contains("projet");
        let dest = Target::new(&orch, &spoken);
        if !dest.here && dest.found.is_none() && !explicit {
            return None;
        }
        if let Some(f) = &dest.found {
            if !explicit && js::gn_or(f, "score", 100.0) < 60.0 {
                return None;
            }
        }
        return Some(it(o, "projects_info", "Projet", &dest.label(), move |o| {
            let dest = dest.clone();
            async move {
                let Some(project) = dest.get(&o).await else { return dest.missing(&o) };
                let name = js::gs(&project, "name");
                let (oc, p2) = (o.self_ref(), project.clone());
                let r = tool(&o, "mcp__system__projets", json!({ "action": "infos", "nom": name }), move |_| async move {
                    let mut info = oc.up().projects.info(&p2).await;
                    info["ok"] = json!(true);
                    info
                })
                .await;
                if let Some(x) = refused(&r) {
                    return x;
                }
                let info = o.projects.info(&project).await;
                let say = o.projects.describe(&info);
                o.show_widget("tool", super::skill_intents::project_info_card(&info));
                json!({ "say": say, "display": name })
            }
        }));
    }
    None
}

// --- Exclusions des annonces de nouveaux mails ---------------------------------------------------------

const MAIL_FROM: &str = "(?:de|du|des|d|venant de|provenant de|de la part de)";
const MAIL_ABOUT: &str = "(?:qui parlent de|qui parle de|dont l objet contient|contenant|avec le mot|avec l objet)";
lazy_re!(mail_exclude_re, format!("^(?:n annonce plus|ne m annonce plus|n affiche plus|ne m affiche plus|ne signale plus|ne me signale plus|ne me previens plus|ne me previens plus|arrete de m annoncer|arrete d annoncer|arrete de me signaler|ignore|exclus|exclue|exclure|masque|bloque|filtre|je ne veux plus (?:etre prevenu|voir|entendre parler))(?: (?:pour|des|de|les|d|moi|plus|mes))* (?:nouveaux |annonces de |annonces des |notifications de |notifications des )?{MAIL} (?:{MAIL_FROM}|{MAIL_ABOUT}) (.+)$"));
lazy_re!(mail_restore_re, format!("^(?:reannonce|annonce de nouveau|annonce a nouveau|annonce moi de nouveau|annonce moi a nouveau|reactive|retablis|ne masque plus|n ignore plus|ne bloque plus|debloque|n exclus plus|previens moi de nouveau|signale de nouveau|reinclus|inclus de nouveau)(?: (?:pour|des|de|les|d|moi|mes))* (?:nouveaux |annonces de |annonces des |notifications de |notifications des )?{MAIL} (?:{MAIL_FROM}|{MAIL_ABOUT}) (.+)$"));

fn mail_filter_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    let raw = raw.trim().to_string();
    // Partie utile de la phrase d'origine : (mot de l'objet, expéditeur).
    let value_of = |raw: &str| -> (Option<String>, Option<String>) {
        if let Some(a) = crate::re!(r"(?i)(?:qui parlent? de|dont l['’]objet contient|contenant|avec le mot|avec l['’]objet)\s+(.+?)\s*[.!?]*$").captures(raw) {
            return (Some(crate::re!(r#"^[«"“]\s*|\s*[»"”]$"#).replace_all(&a[1], "").trim().to_string()), None);
        }
        let from = crate::re!(r"(?i)(?:e-?mails?|mails?|courriels?|messages?)\s+(?:de la part de|venant de|provenant de|des|du|de|d['’])\s*(.+?)\s*[.!?]*$").captures(raw).map(|f| f[1].trim().to_string());
        (None, from)
    };
    if mail_exclude_re().is_match(t) {
        let (kw, from) = value_of(&raw);
        let label = kw.clone().or(from.clone()).unwrap_or_default();
        return Some(it(o, "mail_exclude", "Exclusion de mails", &label.clone(), move |o| {
            let (kw, from, label) = (kw.clone(), from.clone(), label.clone());
            async move {
                if label.is_empty() {
                    return json!({ "say": "De quel expéditeur ? Dis par exemple : n'annonce plus les mails de news@amazon.fr.", "display": "Expéditeur manquant" });
                }
                let r = match &kw {
                    Some(k) => o.mail_filters.add(&json!({ "type": "keyword", "value": super::projects::fold_name(k), "label": k })),
                    None => o.mail_filters.add(&json!(from)),
                };
                if !js::gb(&r, "ok") {
                    return json!({ "say": "Je n'ai pas compris quel expéditeur exclure.", "display": "Exclusion impossible" });
                }
                let rule = &r["rule"];
                let l = js::gs(rule, "label");
                let what = if js::gs(rule, "type") == "keyword" { format!("les mails qui parlent de {l}") } else { format!("les mails de {l}") };
                json!({
                    "say": if js::gb(&r, "existed") { format!("Je n'annonçais déjà plus {what}.") } else { format!("C'est noté : je ne t'annoncerai plus {what}. Je l'ai ajouté à ma mémoire.") },
                    "display": format!("Exclu : {l}")
                })
            }
        }));
    }
    if mail_restore_re().is_match(t) {
        let (kw, from) = value_of(&raw);
        let label = kw.or(from).unwrap_or_default();
        return Some(it(o, "mail_restore", "Exclusion de mails", &label.clone(), move |o| {
            let label = label.clone();
            async move {
                let removed = o.mail_filters.remove(&label);
                if removed.is_empty() {
                    return json!({ "say": format!("Je n'avais pas d'exclusion pour {label}."), "display": "Aucune exclusion" });
                }
                let names: Vec<String> = removed.iter().map(|r| js::gs(r, "label")).collect();
                json!({ "say": format!("D'accord, je t'annonce de nouveau les mails de {}.", names.join(", ")), "display": format!("Réactivé : {}", names.join(", ")) })
            }
        }));
    }
    if crate::re!(r"^(?:(?:liste|montre|affiche|donne|lis)(?: moi)? (?:mes|les) (?:exclusions|filtres)(?: (?:de|des|sur les))?(?: mails| e mails| courriels)?|(?:quels|quelles) (?:sont )?(?:les |mes )?(?:mails|expediteurs|exclusions|filtres)(?: de mails)? (?:exclus|ignores|masques|bloques|filtres|que tu n annonces (?:pas|plus))|quels mails (?:tu )?n annonces (?:tu )?(?:pas|plus))$").is_match(t) {
        return Some(it(o, "mail_exclusions", "Exclusions de mails", "liste", |o| async move {
            let rules = o.mail_filters.list();
            let kind = |k: &str| match k {
                "from" => "adresse",
                "domain" => "domaine",
                "sender" => "expéditeur",
                "keyword" => "mot de l’objet",
                _ => "",
            };
            let rows: Vec<J> = rules
                .iter()
                .map(|r| {
                    let (typ, l) = (js::gs(r, "type"), js::gs(r, "label"));
                    let what = if typ == "keyword" { format!("les mails qui parlent de {l}") } else { format!("les mails de {l}") };
                    let say = if typ == "keyword" { format!("Annonce de nouveau les mails qui parlent de {l}") } else { format!("Annonce de nouveau les mails de {}", if typ == "from" || typ == "domain" { js::gs(r, "value") } else { l.clone() }) };
                    json!({ "icon": "🚫", "title": l, "sub": if kind(&typ).is_empty() { typ.clone() } else { kind(&typ).to_string() }, "actions": [{ "icon": "↺", "title": format!("Annoncer de nouveau {what}"), "say": say, "done": "réactivé" }] })
                })
                .collect();
            o.show_widget(
                "tool",
                json!({
                    "title": "Mails non annoncés", "tool": "mail", "count": rules.len(), "ttl": 120000,
                    "empty": "Aucune exclusion : tous les nouveaux mails sont annoncés.", "rows": rows,
                    "footer": "↺ : annoncer de nouveau · aussi dans Administration › Mémoire (catégorie « mails »)"
                }),
            );
            let n = rules.len();
            json!({
                "say": if n > 0 { format!("{n} exclusion{} : {}.", p_plural(n), rules.iter().take(5).map(|r| js::gs(r, "label")).collect::<Vec<_>>().join(", ")) } else { "Aucune exclusion : je t’annonce tous les nouveaux mails.".into() },
                "display": format!("{n} exclusions")
            })
        }));
    }
    None
}

/// Les étapes qui passeront par l'IA (aucune intention locale ne les reconnaît).
fn via_ai(router: &Router, steps: &[String]) -> Vec<String> {
    steps
        .iter()
        .filter(|s| match router.route(s, false) {
            None => true,
            Some(r) => r.ignored || r.kind.starts_with("small_talk"),
        })
        .cloned()
        .collect()
}

fn slot_for_card_number(o: &Orchestrator, n: i64) -> Option<String> {
    if let Some(m) = o.status.d.borrow().card_numbers.as_object() {
        for (slot, num) in m {
            if num.as_i64() == Some(n) {
                return Some(slot.clone());
            }
        }
    }
    if let Some(slot) = crate::modules::number_default_slot(n) {
        return Some(slot);
    }
    if let Some((slot, _)) = super::cards::DEFAULT_SLOTS.iter().find(|(_, num)| *num == n) {
        return Some((*slot).to_string());
    }
    Some(
        match n {
            1 => "bureau",
            2 => "audio",
            3 => "hw",
            4 => "conversation",
            5 => "actions",
            110 => "savoir",
            114 => "tool:Checkup",
            115 => "diction",
            116 => "devant",
            120 => "nomade_view",
            121 => "creer-module",
            122 => "agents",
            124 => "amis_view",
            _ => return None,
        }
        .to_string(),
    )
}

pub(crate) fn slot_for_spoken_module(o: &Orchestrator, spoken: &str) -> Option<String> {
    let s = crate::re!(r"\s+(?:s il te plait|s il vous plait|stp|svp|merci)$").replace(spoken.trim(), "");
    let s = crate::re!(r"\s+(?:numero\s+|n\s+|no\s+)?\d{1,3}$").replace(s.trim(), "");
    let s = crate::re!(r"^(?:de |des |du |d |la |le |les |l |mes |mon |ma |toutes mes |tous mes |toutes les |tous les )+").replace(s.trim(), "");
    let s = s.trim();
    let known = match s {
        "liste" | "listes" | "list" | "lists" | "collection" | "collections" | "listes a fiches" | "liste a fiches" | "listes a cocher" | "liste a cocher" => Some("tool:Mes listes"),
        "nomade" | "nomad" | "site nomade" | "site nomad" | "depots nomade" | "depots nomad" | "depot nomade" | "depot nomad" => Some("nomade_view"),
        // « ami » seul, comme « messages », désigne la card Amis (le « s » final est déjà retiré ou non).
        "amis" | "ami" | "mes amis" | "amis jarvis" | "messages" | "messagerie" => Some("amis_view"),
        "notes" | "note" | "prise de notes" | "prise de note" => Some("tool:notes"),
        "reunion" | "reunions" | "visio" | "visios" | "gmeet" | "google meet" => Some("tool:reunions"),
        "projet" | "projets" => Some("tool:projets"),
        "data" | "savoir" | "dossier data" | "dossiers data" | "dossier de recherche" | "dossiers de recherche" => Some("savoir"),
        "mail" | "mails" | "email" | "emails" | "e mail" | "e mails" | "courriel" | "courriels" | "gmail" | "boite mail" => Some("mail"),
        "agenda" | "calendrier" | "google agenda" | "rendez vous" | "rdv" | "planning" => Some("agenda"),
        "checkup" | "check up" | "bilan de sante" | "etat de sante" | "fonctions vitales" => Some("tool:Checkup"),
        "diction" | "prononciation" => Some("diction"),
        "devant" | "premier plan" | "fenetres devant" => Some("devant"),
        "bureau" => Some("bureau"),
        "create module" | "creer module" | "creer un module" | "creation de module" | "creation de modules" | "studio de modules" | "module studio" => Some("creer-module"),
        "agent" | "agents" | "agent autonome" | "agents autonomes" => Some("agents"),
        "terminal" | "terminaux" => Some("tool:Terminal"),
        "action" | "actions" | "menu actions" => Some("actions"),
        "audio" | "son" | "voix" => Some("audio"),
        "materiel" | "hardware" | "hw" | "telemetrie" => Some("hw"),
        "conversation" | "chat" | "discussion" => Some("conversation"),
        "raccourci" | "raccourcis" | "macro" | "macros" => Some("tool:raccourcis"),
        "automatisation" | "automatisations" => Some("tool:automatisations"),
        "programmation" | "programmations" => Some("tool:programmations"),
        "rappel" | "rappels" => Some("tool:rappels"),
        _ => None,
    };
    if let Some(k) = known {
        return Some(k.to_string());
    }
    let slug = crate::module_studio::slug(s);
    if !slug.is_empty() {
        let user_root = crate::modules::user_dir(&o.data_dir);
        let (mods, _) = crate::modules::scan_roots(&[(user_root, true)]);
        for m in mods {
            let id = js::gs(&m, "id");
            let nom_slug = crate::module_studio::slug(&js::gs(&m, "nom"));
            if id == slug || id == format!("perso-{slug}") || nom_slug == slug {
                return Some(id);
            }
        }
    }
    None
}

fn open_slot_intent(o: &OrchRef, slot: &str) -> Option<Intent> {
    match slot {
        "tool:Mes listes" | "listes" => Some(it(o, "collections_overview", "Mes listes", "toutes les listes", |o| async move {
            o.collections.show_overview();
            let (f, c) = (o.collections.count(), o.smart_lists.count());
            let names: Vec<String> = o.collections.all().iter().map(|l| js::gs(l, "nom")).collect();
            let text = match (f, c) {
                (0, 0) => "Tu n'as encore aucune liste. Dis par exemple : crée-moi une liste de disques vinyle.".to_string(),
                (0, _) => format!("Tu as {} liste{} à cocher.", c, p_plural(c)),
                (_, 0) => format!("Tu as {} liste{} : {}.", f, p_plural(f), names.join(", ")),
                _ => format!("Tu as {} liste{} à fiches : {}, et {} liste{} à cocher.", f, p_plural(f), names.join(", "), c, p_plural(c)),
            };
            json!({ "say": text, "display": format!("{} listes", f + c) })
        })),
        "amis_view" | "amis" => Some(it(o, "amis_show", "Amis", "Jarvis Réseau : amis, messages, partages", |o| async move {
            o.reseau.show();
            let v = o.reseau.view();
            let unread: i64 = js::arr(&v["amis"]).iter().map(|f| js::gn(f, "nonLus").unwrap_or(0.0) as i64).sum();
            let asks = js::arr(&v["amis"]).iter().filter(|f| js::gs(f, "statut") == "pending" && !js::gb(f, "envoyee")).count();
            let plural = |n: i64| if n > 1 { "s" } else { "" };
            let say = if !js::gb(&v, "relie") {
                "Relie d'abord ton compte : je t'ouvre la card Amis.".to_string()
            } else {
                match (unread, asks as i64) {
                    (0, 0) => "Rien de nouveau de tes amis.".to_string(),
                    (u, 0) => format!("{u} message{} non lu{}.", plural(u), plural(u)),
                    (0, a) => format!("{a} demande{} d'ami en attente.", plural(a)),
                    (u, a) => format!("{u} message{} non lu{} et {a} demande{} d'ami.", plural(u), plural(u), plural(a)),
                }
            };
            json!({ "say": say, "display": "Amis" })
        })),
        "nomade_view" | "nomade" => Some(it(o, "nomade_show", "Nomade", "dépôts et état du site", |o| async move {
            let n = js::arr(&o.nomade.state()["depots"]).len();
            o.nomade.show();
            let say = match n {
                0 => "Rien n'a été déposé depuis le site Nomade.".to_string(),
                1 => "Un dépôt t'attend depuis le site Nomade.".to_string(),
                n => format!("{n} dépôts t'attendent depuis le site Nomade."),
            };
            json!({ "say": say, "display": "Nomade" })
        })),
        "tool:notes" | "notes" | "note" => {
            let only_cur = slot == "note";
            Some(it(o, "notes_list", "Mes notes", "liste", move |o| async move {
                if only_cur && o.notes.current.borrow().is_some() {
                    o.notes.show();
                    return json!({ "say": format!("J'affiche la note {}.", o.notes.current_title()), "display": o.notes.current_title() });
                }
                let all = o.notes.show_list();
                let n = all.len();
                json!({
                    "say": if n > 0 { format!("Tu as {n} note{}. La plus récente : {}.", p_plural(n), all[0].title) } else { "Tu n’as encore aucune note enregistrée.".into() },
                    "display": format!("{n} notes")
                })
            }))
        }
        "tool:reunions" | "reunions" | "meeting" => {
            let only_cur = slot == "meeting";
            Some(it(o, "meetings_list", "Mes réunions", "liste", move |o| async move {
                if only_cur && o.meetings.session.borrow().is_some() {
                    o.meetings.show();
                    return json!({ "say": "J'affiche la réunion en cours.", "display": "Réunion" });
                }
                let all = o.meetings.show_list("Mes réunions");
                let n = all.len();
                let p = p_plural(n);
                json!({
                    "say": if n > 0 { format!("Tu as {n} réunion{p} enregistrée{p}. La plus récente : {}.", js::gs(&all[0], "title")) } else { "Aucune réunion enregistrée pour l’instant.".into() },
                    "display": format!("{n} réunions")
                })
            }))
        }
        "tool:projets" | "projets" => project_intents(o, "mes projets"),
        "savoir" | "savoir_list" => Some(it(o, "savoir", "Dossiers Data", "liste", |o| async move {
            o.savoir.show_list();
            let n = o.savoir.list().len();
            json!({
                "say": if n == 0 { "Tu n'as encore aucun dossier. Dis par exemple : fais un dossier sur l'hélicoptère H160.".to_string() } else { format!("Tu as {n} dossier{}.", p_plural(n)) },
                "display": "Dossiers Data"
            })
        })),
        "mail" | "mails" => mail_intents(o, "mes derniers mails"),
        "agenda" => Some(it(o, "agenda_card", "Agenda", "prochains rendez-vous", |o| async move {
            let _ = o.list_cards.next_appointments().await;
            json!({ "say": "Voici ton agenda.", "display": "Agenda" })
        })),
        "tool:Checkup" | "checkup" => Some(it(o, "checkup", "Checkup", "scan des fonctions vitales", |o| async move {
            o.show_checkup_card();
            let r = o.checkup_report();
            json!({ "say": Orchestrator::checkup_summary(&r), "display": format!("Checkup : {} %", js::s(&r["score"])) })
        })),
        "diction" | "tool:Diction" => Some(it(o, "diction_open", "Diction", "corriger une prononciation", |o| async move {
            let v = super::pronunciation::diction_op(&o, "ouvrir", &json!({}));
            let n = js::arr(&v["phrases"]).len();
            json!({ "say": if n == 0 { "Je n'ai encore rien dit à corriger.".to_string() } else { "Touche le mot que j'ai mal dit.".to_string() }, "display": "Diction" })
        })),
        "devant" | "tool:Devant" => Some(it(o, "devant_open", "Devant", "applications au premier plan", |o| async move {
            o.show_devant_card();
            json!({ "say": "Voici les applications ouvertes.", "display": "Devant" })
        })),
        "bureau" => Some(it(o, "bureau_card", "Bureau", "applications ouvertes", |o| async move {
            let card = super::desktop_intents::desktop_card(&o, None, false);
            o.show_widget("tool", card);
            json!({ "say": "Voici la card Bureau.", "display": "Bureau" })
        })),
        "creer-module" => Some(it(o, "creer_module_open", "Create module", "atelier de modules", |o| async move {
            o.emit(json!({ "type": "creer_module_open" }));
            json!({ "say": "J'ouvre l'atelier de création de modules.", "display": "Create module" })
        })),
        "agents" => {
            let orch = o.up();
            orch.agents.match_("ouvre mes agents", "ouvre mes agents")
        }
        "tool:Terminal" | "terminal" => Some(it(o, "terminal_card", "Terminal", "terminaux et agents", |o| async move {
            let n = o.status.d.borrow().card_numbers["tool:Terminal"].as_i64().unwrap_or(101);
            let r = o.cards.open(n).await;
            if js::gb(&r, "ok") {
                return json!({ "say": null, "display": "Terminal affiché" });
            }
            o.terminal_voice.update_ai_status();
            o.tmux_voice.refresh_cards();
            json!({ "say": "Voici la card Terminal.", "display": "Terminal" })
        })),
        "audio" | "hw" | "materiel" | "conversation" | "actions" => {
            let key = if slot == "materiel" { "hw" } else { slot }.to_string();
            let label = match key.as_str() {
                "audio" => "Audio",
                "hw" => "Matériel",
                "conversation" => "Conversation",
                _ => "Actions",
            };
            Some(it(o, "card_expand", label, "ouverture", move |o| {
                let key = key.clone();
                async move {
                    o.emit(json!({ "type": "card_expand", "key": key, "collapsed": false }));
                    json!({ "say": format!("J'ouvre la card {label}."), "display": label })
                }
            }))
        }
        "tool:raccourcis" => Some(it(o, "macro_list", "Raccourcis", "liste", |o| async move {
            let list = o.macros.list();
            o.list_cards.macros_card();
            json!({ "say": if list.is_empty() { "Tu n'as aucun raccourci.".to_string() } else { format!("{} raccourci{}.", list.len(), p_plural(list.len())) }, "display": "Raccourcis" })
        })),
        "tool:automatisations" => Some(it(o, "automation_list", "Automatisations", "liste", |o| async move {
            let n = o.automations.rules().len();
            o.list_cards.automations_card();
            json!({ "say": if n == 0 { "Tu n'as aucune automatisation.".to_string() } else { format!("{n} automatisation{}.", p_plural(n)) }, "display": "Automatisations" })
        })),
        "tool:programmations" => schedule_intents(o, "mes programmations", "mes programmations"),
        "tool:rappels" => reminder_intents(o, "mes rappels", "mes rappels"),
        s if s.starts_with("perso-") => {
            let key = s.to_string();
            Some(it(o, "card_expand", "Module", s, move |o| {
                let key = key.clone();
                async move {
                    o.emit(json!({ "type": "card_expand", "key": key, "collapsed": false }));
                    json!({ "say": "J'ouvre le module.", "display": key })
                }
            }))
        }
        _ => None,
    }
}

fn close_slot_intent(o: &OrchRef, slot: &str) -> Intent {
    let slot = slot.to_string();
    it(o, "card_close", "Card", &slot.clone(), move |o| {
        let slot = slot.clone();
        async move {
            if matches!(slot.as_str(), "audio" | "hw" | "materiel" | "conversation" | "actions") || slot.starts_with("perso-") {
                let key = if slot == "materiel" { "hw" } else { slot.as_str() };
                o.emit(json!({ "type": "card_expand", "key": key, "collapsed": true }));
                return json!({ "say": "Card réduite.", "display": "Card réduite" });
            }
            if matches!(slot.as_str(), "creer-module" | "agents") {
                o.emit(json!({ "type": "widget_close", "key": slot }));
                return json!({ "say": "Card fermée.", "display": "Card fermée" });
            }
            let n = o.cards.by_key.borrow().iter().find(|c| c.slot == slot || c.key == slot).map(|c| c.n);
            if let Some(n) = n {
                let r = o.cards.close(n, false).await;
                return json!({ "say": r["say"], "display": r["say"] });
            }
            o.emit(json!({ "type": "widget_close", "key": slot }));
            json!({ "say": "Card fermée.", "display": "Card fermée" })
        }
    })
}

pub fn skill_intent(router: &Router, text: &str, raw: &str) -> Option<Intent> {
    let o = router.orch().clone();
    let orch = o.up();
    let t = crate::re!(r"[.…]+").replace_all(text, " ");
    let t = crate::re!(r"\s+").replace_all(&t, " ").trim().to_string();
    let t = t.as_str();

    // --- Macros personnelles -------------------------------------------------------------------------
    let raw_clean = crate::re!(r"[.!?]+$").replace(raw.trim(), "").into_owned();
    let mk = crate::re!(r#"(?i)^(?:jarvis[, ]+)?quand je (?:dis|dirai|te dis|te dirai)\s+[«"“]?\s*(.+?)\s*[»"”]?\s*[,:]\s*(?:tu\s+)?(?:fais|feras|alors)?\s*:?\s*(.+)$"#).captures(&raw_clean);
    if let Some(m) = &mk {
        let trigger = m[1].trim().to_string();
        let steps = super::macros::split_steps(&m[2]);
        let ai = via_ai(router, &steps);
        return Some(it(&o, "macro_create", "Nouveau raccourci", &trigger.clone(), move |o| {
            let (trigger, steps, ai) = (trigger.clone(), steps.clone(), ai.clone());
            async move {
                if steps.is_empty() {
                    return json!({ "say": "Je n'ai pas compris les actions de ce raccourci.", "display": "Raccourci vide" });
                }
                o.macros.add(&trigger, steps.clone());
                o.record("macro_creee", json!({ "nom": trigger, "etapes": steps, "via_ia": ai }));
                let (oc, t2) = (o.self_ref(), trigger.clone());
                o.undo.add(&format!("raccourci « {trigger} »"), Box::new(move || Box::pin(async move { if oc.up().macros.remove(&t2) > 0 { format!("Raccourci « {t2} » supprimé.") } else { "Ce raccourci n’existe plus.".to_string() } })));
                let warn = if ai.is_empty() {
                    " Tout se fera sans IA.".to_string()
                } else {
                    format!(" Attention : {} passer{} par l'IA.", ai.iter().map(|s| format!("« {s} »")).collect::<Vec<_>>().join(", "), if ai.len() > 1 { "ont" } else { "a" })
                };
                json!({ "say": format!("Raccourci « {trigger} » créé, {} étape{}.{warn}", steps.len(), p_plural(steps.len())), "display": format!("{trigger} · {} étapes", steps.len()) })
            }
        }));
    }
    // --- Automatisations « quand X, fais Y » -----------------------------------------------------------
    if let Some(rule) = super::automations::parse_rule(&raw_clean) {
        let steps = js::str_list(&rule["steps"]);
        let ai = via_ai(router, &steps);
        let target = js::gs(&rule["trigger"], "target");
        return Some(it(&o, "automation_create", "Nouvelle automatisation", &target, move |o| {
            let (rule, ai) = (rule.clone(), ai.clone());
            async move {
                let added = o.automations.add(&rule);
                let label = o.automations.label(&added);
                o.record("automatisation_creee", json!({ "regle": label, "via_ia": ai }));
                let (oc, id) = (o.self_ref(), js::gs(&added, "id"));
                o.undo.add("automatisation", Box::new(move || Box::pin(async move { if oc.up().automations.remove(&id) { "Automatisation supprimée.".to_string() } else { "Cette automatisation n’existe plus.".to_string() } })));
                o.list_cards.automations_card();
                let warn = if ai.is_empty() { String::new() } else { format!(" {} passer{} par l'IA.", ai.iter().map(|s| format!("« {s} »")).collect::<Vec<_>>().join(", "), if ai.len() > 1 { "ont" } else { "a" }) };
                json!({ "say": format!("C'est noté : {}.{warn}", label.replacen('→', ", je ferai :", 1)), "display": label })
            }
        }));
    }
    if crate::re!(r"\b(?:liste|quelles sont|montre|affiche|donne) (?:moi )?(?:mes |les )?automatisations?\b").is_match(t) {
        return Some(it(&o, "automation_list", "Automatisations", "liste", |o| async move {
            let n = o.automations.rules().len();
            o.list_cards.automations_card();
            if n == 0 {
                return json!({ "say": "Tu n'as aucune automatisation. Dis par exemple : quand Teams s'ouvre, mets la musique en pause.", "display": "0 automatisation" });
            }
            json!({ "say": format!("{n} automatisation{}. La liste est à l'écran.", p_plural(n)), "display": format!("{n} automatisations") })
        }));
    }
    if let Some(m) = crate::re!(r"^(supprime|oublie|efface|retire|lance|execute|mets en pause|suspends|desactive|reactive|active|reprends) (?:l ?['’]?\s?|mon |ma )?automatisation (?:numero |n )?(\d{1,2}|un|une|deux|trois|quatre|cinq)$").captures(t) {
        let n = small_num(&m[2]).unwrap_or(0);
        let action = match &m[1] {
            "supprime" | "oublie" | "efface" | "retire" => "supprimer",
            "lance" | "execute" => "lancer",
            "reactive" | "active" | "reprends" => "reprendre",
            _ => "pause",
        };
        return Some(it(&o, &format!("automation_{action}"), "Automatisation", &format!("n° {n}"), move |o| async move {
            let id = o.list_cards.by_number("automatisation", n).or_else(|| n.checked_sub(1).and_then(|i| o.automations.rules().get(i as usize).map(|r| r["id"].clone())));
            let Some(id) = id else { return json!({ "say": format!("Il n'y a pas d'automatisation {n}. Dis « liste mes automatisations »."), "display": "Introuvable" }) };
            let r = o.list_cards.action("automatisation", &id, action).await;
            json!({ "say": r["say"], "display": format!("n° {n}") })
        }));
    }
    if crate::re!(r"\b(?:liste|quels sont|montre|affiche|donne) (?:moi )?(?:mes |les )?(?:raccourcis|macros)\b").is_match(t) {
        return Some(it(&o, "macro_list", "Raccourcis", "liste", |o| async move {
            let list = o.macros.list();
            o.list_cards.macros_card();
            if list.is_empty() {
                return json!({ "say": "Tu n'as aucun raccourci. Dis par exemple : quand je dis mode travail, ouvre Visual Studio Code et mets de la musique.", "display": "0 raccourci" });
            }
            let names: Vec<String> = list.iter().map(|m| js::gs(m, "name")).collect();
            json!({ "say": format!("{} raccourci{} : {}. La liste est à l'écran.", list.len(), p_plural(list.len()), names.join(", ")), "display": names.join(", ") })
        }));
    }
    if let Some(m) = crate::re!(r"^(lance|execute|active|supprime|oublie|efface|retire) (?:le |la |mon |ma )?(?:raccourci|macro) (?:numero |n )?(\d{1,2}|un|deux|trois|quatre|cinq)$").captures(t) {
        let n = small_num(&m[2]).unwrap_or(0);
        let del = matches!(&m[1], "supprime" | "oublie" | "efface" | "retire");
        return Some(it(&o, if del { "macro_delete" } else { "macro_run" }, "Raccourci", &format!("n° {n}"), move |o| async move {
            let name = o.list_cards.by_number("raccourci", n).or_else(|| n.checked_sub(1).and_then(|i| o.macros.list().get(i as usize).map(|m| m["name"].clone())));
            let Some(name) = name else { return json!({ "say": format!("Il n'y a pas de raccourci {n}. Dis « liste mes raccourcis »."), "display": "Introuvable" }) };
            let r = o.list_cards.action("raccourci", &name, if del { "supprimer" } else { "lancer" }).await;
            json!({ "say": r["say"], "display": name })
        }));
    }
    if let Some(m) = crate::re!(r"^(?:supprime|oublie|efface|retire) (?:le |la |mon |ma )?(?:raccourci|macro) (.+)$").captures(t) {
        let name = m[1].to_string();
        return Some(it(&o, "macro_delete", "Raccourci supprimé", &name.clone(), move |o| {
            let name = name.clone();
            async move {
                let n = o.macros.remove(&name);
                if n > 0 {
                    o.record("macro_supprimee", json!({ "nom": name }));
                }
                json!({ "say": if n > 0 { format!("Raccourci {name} supprimé.") } else { format!("Je ne trouve pas de raccourci {name}.") }, "display": name })
            }
        }));
    }

    // --- Réunions, notes et listes ------------------------------------------------------------------------
    if let Some(i) = orch.meetings.match_(t, raw) {
        return Some(i);
    }
    if let Some(i) = orch.notes.match_(t, raw) {
        return Some(i);
    }
    if let Some(i) = super::collections::voice::match_collection(&o, t, raw) {
        return Some(i);
    }
    if let Some(i) = super::smart_lists::match_smart_list(&o, t, raw) {
        return Some(i);
    }

    // --- Dessin dans Paint -----------------------------------------------------------------------------
    if let Some(d) = super::drawing::parse_draw_request(t) {
        if let Some(key) = d["key"].as_str().map(String::from) {
            let dr = super::drawing::drawing(&key)?;
            let (name, say) = (dr.name.clone(), dr.say.clone().unwrap_or_default());
            let random = js::gb(&d, "random");
            let mut i = it(&o, "draw_paint", &format!("Dessin : {name}"), "Paint", move |o| {
                let (key, name, say) = (key.clone(), name.clone(), say.clone());
                async move {
                    let ws = crate::paths::workspace().to_string_lossy().to_string();
                    let r = sys_tool(&o, "mcp__system__dessiner", "dessiner", json!({ "sujet": key, "workspace": ws })).await;
                    if let Some(x) = refused(&r) {
                        return x;
                    }
                    if !js::truthy(&r) || r["ok"] == json!(false) || js::truthy(&r["error"]) {
                        let e = js::gs(&r, "error");
                        return json!({ "say": format!("Je n'ai pas réussi à dessiner {name}.{}", if e.is_empty() { String::new() } else { format!(" {e}") }), "display": "Dessin impossible" });
                    }
                    json!({ "say": if random { format!("J'ai choisi {name}. {say}") } else { say }, "display": format!("{name} · Paint") })
                }
            });
            i.covers = vec!["mspaint".into()];
            return Some(i);
        }
        let spoken = js::gs(&d, "spoken");
        let detail = if spoken.is_empty() { "sujet inconnu".to_string() } else { spoken.clone() };
        let mut i = it(&o, "draw_unknown", "Paint", &detail, move |o| {
            let spoken = spoken.clone();
            async move {
                let r = sys_tool(&o, "mcp__system__ouvrir_application", "ouvrir_application", json!({ "nom": "mspaint" })).await;
                if let Some(x) = refused(&r) {
                    return x;
                }
                let what = if spoken.is_empty() { "dessiner ça".to_string() } else { format!("dessiner {spoken}") };
                let keys: Vec<String> = super::drawing::DRAWINGS.iter().map(|d| d.key.clone()).collect();
                json!({ "say": format!("Je ne sais pas encore {what}. J'ouvre Paint. Je sais dessiner, par exemple, {}.", super::drawing::subject_list(5)), "display": format!("Sujets : {}", keys.join(", ")) })
            }
        });
        i.covers = vec!["mspaint".into()];
        return Some(i);
    }

    // --- Fichiers : « ouvre le 2 » après une recherche --------------------------------------------------
    if let Some(m) = crate::re!(r"^(?:ouvre|lance|affiche|montre)(?: moi)? (?:le |la )?(?:numero |n )?(\d{1,2}|premier|deuxieme|second|troisieme|quatrieme|cinquieme|sixieme|dernier)(?: fichier| resultat| dossier)?$").captures(t) {
        let last = orch.last_file_results.borrow().clone();
        if let Some((items, at)) = last.filter(|(_, at)| js::now_ms() - at < 15 * 60 * 1000) {
            let _ = at;
            let mut n = m[1].parse::<i64>().ok().or_else(|| ordinal(&m[1])).unwrap_or(0);
            if n == -1 {
                n = items.len() as i64;
            }
            if let Some(target) = n.checked_sub(1).and_then(|i| items.get(i as usize).cloned()) {
                let name = basename(&target);
                return Some(it(&o, "file_open", "Ouverture", &name.clone(), move |o| {
                    let (target, name) = (target.clone(), name.clone());
                    async move {
                        let oc = o.self_ref();
                        let r = tool(&o, "mcp__system__ouvrir_fichier", json!({ "chemin": target }), move |p| async move { sysr(oc.up().file_search.open(&js::gs(&p, "chemin"))) }).await;
                        if let Some(x) = refused(&r) {
                            return x;
                        }
                        if js::truthy(&r["error"]) {
                            return json!({ "say": format!("Impossible d'ouvrir ce fichier : {}", js::gs(&r, "error")), "display": "Échec" });
                        }
                        json!({ "say": if js::gb(&r, "executable") { "C'est un programme : je ne le lance pas, j'affiche son dossier.".to_string() } else { format!("J'ouvre {name}.") }, "display": name })
                    }
                }));
            }
        }
    }

    // --- Dossiers personnels connus ------------------------------------------------------------------
    if let Some(m) = crate::re!(r"^(?:ouvre|affiche|montre|va dans)(?: moi)? (?:le dossier |le repertoire |mes |mon |ma |les |le |la )?(bureau|documents|telechargements|images|photos|musique|videos)$").captures(t) {
        let key = m[1].to_string();
        let dir = super::file_search::known_folder(&key).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        return Some(it(&o, "folder_open", "Dossier", &key.clone(), move |o| {
            let (key, dir) = (key.clone(), dir.clone());
            async move {
                let r = sys_tool(&o, "mcp__system__ouvrir_dossier", "ouvrir_dossier", json!({ "chemin": dir })).await;
                if let Some(x) = refused(&r) {
                    return x;
                }
                json!({ "say": if js::truthy(&r["error"]) { "Je n'arrive pas à ouvrir ce dossier.".to_string() } else { format!("J'ouvre {}.", folder_name(&key)) }, "display": dir })
            }
        }));
    }

    // --- Recherche de fichiers / dossiers ------------------------------------------------------------
    if let Some(m) = crate::re!(r"^(?:trouve|cherche|recherche|retrouve|localise|ou est|ou sont|ou se trouve|ou j ai mis|ouvre)(?: moi)? (?:le |la |les |un |une |des |mon |ma |mes )?(fichier|fichiers|document|documents|dossier|dossiers|repertoire|pdf|photos?|images?|video|presentation|tableur|feuille de calcul|cv)(?:\s+(?:qui s appelle|nomme|appele|intitule|de|du|des|sur|avec|pour|contenant))?(?:\s+(.+))?$").captures(t) {
        let type_word = m[1].to_string();
        if m.get(2).is_some() || type_word == "cv" {
            let query = m.get(2).map(|x| x.as_str().to_string()).unwrap_or_else(|| "cv".into());
            let query = crate::re!(r"^(?:le |la |les |l )").replace(&query, "").trim().to_string();
            let wants_open = t.starts_with("ouvre");
            let filter: Option<&'static regex::Regex> = match type_word.as_str() {
                "pdf" => Some(crate::re!(r"(?i)\.pdf$")),
                "photo" | "photos" => Some(crate::re!(r"(?i)\.(?:jpe?g|png|heic|webp|gif)$")),
                "image" | "images" => Some(crate::re!(r"(?i)\.(?:jpe?g|png|heic|webp|gif|bmp|svg)$")),
                "video" => Some(crate::re!(r"(?i)\.(?:mp4|mkv|mov|avi|webm)$")),
                "presentation" => Some(crate::re!(r"(?i)\.(?:pptx?|odp|key)$")),
                "tableur" | "feuille de calcul" => Some(crate::re!(r"(?i)\.(?:xlsx?|ods|csv)$")),
                _ => None,
            };
            let kind = if matches!(type_word.as_str(), "dossier" | "dossiers" | "repertoire") { "folder" } else if filter.is_some() || type_word == "cv" { "file" } else { "any" };
            return Some(it(&o, "file_search", "Recherche de fichiers", &query.clone(), move |o| {
                let query = query.clone();
                async move {
                    let (oc, q2) = (o.self_ref(), query.clone());
                    let r = tool(&o, "mcp__system__chercher_fichiers", json!({ "requete": query }), move |_| async move {
                        let mut items = oc.up().file_search.search(&q2, kind, 20).await;
                        if let Some(f) = filter {
                            items.retain(|p| f.is_match(p));
                        }
                        items.truncate(8);
                        json!({ "ok": true, "items": items })
                    })
                    .await;
                    if let Some(x) = refused(&r) {
                        return x;
                    }
                    let items = js::str_list(&r["items"]);
                    if items.is_empty() {
                        return json!({ "say": format!("Je ne trouve rien qui s'appelle {query} dans tes dossiers."), "display": "Aucun résultat" });
                    }
                    *o.last_file_results.borrow_mut() = Some((items.clone(), js::now_ms()));
                    let where_ = |p: &str| {
                        let dir = Path::new(p).parent().map(|d| d.to_string_lossy().to_lowercase()).unwrap_or_default();
                        for k in super::file_search::KNOWN_FOLDERS {
                            if let Some(f) = super::file_search::known_folder(k) {
                                if dir.starts_with(&f.to_string_lossy().to_lowercase()) {
                                    return format!("dans {}", folder_name(k));
                                }
                            }
                        }
                        format!("dans {}", Path::new(p).parent().map(|d| basename(&d.to_string_lossy())).unwrap_or_default())
                    };
                    o.list_cards.files_card(&query, &items);
                    if items.len() == 1 || wants_open {
                        let target = items[0].clone();
                        let (oc, t2) = (o.self_ref(), target.clone());
                        let r = tool(&o, "mcp__system__ouvrir_fichier", json!({ "chemin": target }), move |_| async move { sysr(oc.up().file_search.open(&t2)) }).await;
                        if let Some(x) = refused(&r) {
                            return x;
                        }
                        let extra = if items.len() > 1 { format!(" J'en ai trouvé {}, les autres sont à l'écran.", items.len()) } else { String::new() };
                        return json!({ "say": format!("J'ouvre {}, {}.{extra}", basename(&target), where_(&target)), "display": basename(&target) });
                    }
                    json!({
                        "say": format!("J'en ai trouvé {}. Le plus récent est {}, {}. Dis « ouvre le 2 » pour ouvrir le deuxième, par exemple.", items.len(), basename(&items[0]), where_(&items[0])),
                        "display": format!("{} résultats", items.len())
                    })
                }
            }));
        }
    }

    // --- Prononciation : « prononce coaster comme côsteur » -----------------------------------------
    {
        let r = crate::re!(r"(?i)^jarvis[,\s]+").replace(raw.trim(), "");
        let r = crate::re!(r"[.!?]+$").replace(&r, "").into_owned();
        let teach = crate::re!(r#"(?i)^(?:(?:à partir de maintenant|désormais|dorénavant),?\s+)?(?:prononce|on prononce|il faut prononcer|tu dois prononcer)\s+[«"“]?\s*(.+?)\s*[»"”]?\s+comme\s+[«"“]?\s*(.+?)\s*[»"”]?$"#).captures(&r)
            .or_else(|| crate::re!(r#"(?i)^[«"“]?\s*(.+?)\s*[»"”]?\s+se prononce\s+[«"“]?\s*(.+?)\s*[»"”]?$"#).captures(&r));
        if let Some(m) = teach.filter(|m| m[1].split_whitespace().count() <= 4) {
            let (word, say) = (m[1].to_string(), m[2].to_string());
            return Some(it(&o, "pronunciation_set", "Prononciation", &format!("{word} → {say}"), move |o| {
                let (word, say) = (word.clone(), say.clone());
                async move {
                    let r = o.pronunciation.set(&word, &say);
                    if js::gb(&r, "ok") {
                        json!({ "say": format!("D'accord, je dirai désormais {word} comme ça."), "display": format!("« {} » se dit « {} »", js::gs(&r, "word"), js::gs(&r, "say")) })
                    } else {
                        json!({ "say": "Je n'ai pas compris le mot ou sa prononciation.", "display": r["error"] })
                    }
                }
            }));
        }
        if crate::re!(r"(?i)^(?:corrige (?:ta|la) (?:prononciation|diction)|(?:ouvre |montre )?(?:la )?(?:card )?diction|tu (?:as )?mal prononc[ée]+(?: .*)?|tu prononces mal(?: .*)?)$").is_match(&r) {
            return Some(it(&o, "diction_open", "Diction", "corriger une prononciation", |o| async move {
                let v = super::pronunciation::diction_op(&o, "ouvrir", &json!({}));
                let n = js::arr(&v["phrases"]).len();
                json!({ "say": if n == 0 { "Je n'ai encore rien dit à corriger.".to_string() } else { "Touche le mot que j'ai mal dit.".to_string() }, "display": "Diction" })
            }));
        }
        if let Some(m) = crate::re!(r#"(?i)^(?:oublie|efface|supprime) (?:la )?prononciation (?:de |du |d['’])\s*[«"“]?\s*(.+?)\s*[»"”]?$"#).captures(&r) {
            let word = m[1].to_string();
            return Some(it(&o, "pronunciation_remove", "Prononciation", &word.clone(), move |o| {
                let word = word.clone();
                async move {
                    let r = o.pronunciation.remove(&word);
                    json!({ "say": if js::gb(&r, "ok") { format!("Prononciation de {word} oubliée.") } else { format!("Je n'avais pas de prononciation pour {word}.") }, "display": word })
                }
            }));
        }
    }

    // --- Cards numérotées et modules : « ferme la card 2 », « ouvre la carte listes », « ouvre le module nomad » ---
    {
        const CARD: &str = "(?:la |le |les |l |mon |ma |mes )?(?:card|cards|carte|cartes|carde|cardes|module|modules|widget|widgets|fiche|fiches|panneau)";
        const NUM: &str = r"(?:numero |n |no )?(\d{1,3}|un|une|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|onze|douze)";
        const TAIL: &str = "(?: s il te plait| s il vous plait| stp| svp| merci)?";
        if crate::re!(r"^(?:ferme|fermer|fermez|cache|masque|enleve|retire|vire)(?: moi)? (?:toutes les|tous les|les) (?:cards|cartes|cardes|modules|widgets|fiches)$").is_match(t) {
            return Some(it(&o, "cards_close_all", "Cards", "toutes", |o| async move {
                let n = o.cards.close_all();
                let kept = o.cards.list().iter().filter(|c| c.app_id.as_ref().map(|id| o.cards.is_locked(id)).unwrap_or(false)).count();
                let keep = if kept > 0 { format!(" {kept} card{} d’application{} reste{} affichée{} tant que l’application est ouverte.", p_plural(kept), p_plural(kept), if kept > 1 { "nt" } else { "" }, p_plural(kept)) } else { String::new() };
                json!({
                    "say": if n > 0 { format!("{n} card{} fermée{}. Les applications restent ouvertes.{keep}", p_plural(n), p_plural(n)) } else { format!("{}{keep}", if kept > 0 { "Aucune card à fermer." } else { "Aucune card ouverte." }) },
                    "display": format!("{n} cards fermées")
                })
            }));
        }
        let re = |p: String| regex::Regex::new(&p).unwrap();
        let hide_one = re(format!("^(?:cache|masque|range|reduis)(?: moi)? {CARD} {NUM}{TAIL}$")).captures(t);
        let close_one = re(format!("^(?:ferme|fermer|fermez|supprime|enleve|retire|vire|tue)(?: moi)? {CARD} {NUM}(?: et (?:l |son |sa )?(?:application|appli|programme|terminal))?{TAIL}$")).captures(t);
        let open_app_only = crate::re!(r"\b(?:l application de|l appli de)\b").is_match(t);
        let open_one = re(format!("^(?:ouvre|ouvrez|ouvrir|affiche|affichez|afficher|montre|montrez|montrer|active|activer|lance|lancez|lancer|demarre|demarrer|fais voir)(?: moi)? (?:l application de |l appli de )?{CARD} {NUM}{TAIL}$")).captures(t);
        let with_app = close_one.is_some();
        let opening = open_one.is_some();
        if let Some(n) = hide_one.or(close_one).or(open_one).and_then(|m| super::cards::card_number(&m[1])) {
            if opening {
                if !open_app_only {
                    let custom_slot = orch.status.d.borrow().card_numbers.as_object().and_then(|m| m.iter().find(|(_, v)| v.as_i64() == Some(n)).map(|(k, _)| k.clone()));
                    if let Some(intent) = custom_slot.filter(|s| s != "tool:Terminal" || orch.cards.find(n).and_then(|c| c.app_id).is_none()).and_then(|s| open_slot_intent(&o, &s)) {
                        return Some(intent);
                    }
                    if orch.cards.find(n).and_then(|c| c.app_id).is_none() {
                        if let Some(intent) = slot_for_card_number(&orch, n).and_then(|s| open_slot_intent(&o, &s)) {
                            return Some(intent);
                        }
                    }
                }
                return Some(it(&o, "card_open", "Card", &format!("n° {n}"), move |o| async move {
                    let r = o.cards.open(n).await;
                    if js::gb(&r, "ok") { json!({ "say": null, "display": format!("Card {n} : application affichée") }) } else { json!({ "say": r["say"], "display": r["say"] }) }
                }));
            }
            return Some(it(&o, "card_close", "Card", &format!("n° {n}"), move |o| async move {
                let r = o.cards.close(n, with_app).await;
                json!({ "say": r["say"], "display": r["say"] })
            }));
        }
        if let Some(m) = re(format!("^(?:ferme|fermer|fermez|cache|masque|range|reduis|supprime|enleve|retire|vire)(?: moi)? {CARD} (.+?){TAIL}$")).captures(t) {
            if let Some(slot) = slot_for_spoken_module(&orch, &m[1]) {
                return Some(close_slot_intent(&o, &slot));
            }
        }
        if let Some(m) = re(format!("^(?:(?:ouvre|ouvrez|ouvrir|affiche|affichez|afficher|montre|montrez|montrer|active|activer|lance|lancez|lancer|demarre|demarrer|fais voir)(?: moi)? )?{CARD} (.+?){TAIL}$")).captures(t) {
            if let Some(slot) = slot_for_spoken_module(&orch, &m[1]) {
                if let Some(intent) = open_slot_intent(&o, &slot) {
                    return Some(intent);
                }
            }
        }
    }

    // --- Bureau en direct : fenêtres, écrans, instances de Chrome, actions ------------------------------
    if let Some(i) = super::desktop_intents::desktop_intents(&o, t, raw) {
        return Some(i);
    }
    // --- Briefing du matin --------------------------------------------------------------------------
    if super::briefing::is_briefing(t) || (super::briefing::is_greeting(t) && orch.briefing.wants_auto()) {
        return Some(orch.briefing.intent(""));
    }
    // --- Compétences déposées par l'utilisateur (skills/*.js) -------------------------------------------
    if let Some(i) = orch.skills.match_(t, raw) {
        return Some(i);
    }
    // --- « Défais » ---------------------------------------------------------------------------------
    if super::undo::is_undo(t) {
        return Some(it(&o, "undo", "Annulation", "dernière action", |o| async move { o.undo.undo_last().await }));
    }
    if let Some(i) = parcel_intents(&o, t) {
        return Some(i);
    }
    if let Some(i) = super::nomade::intents(&o, t) {
        return Some(i);
    }
    if let Some(i) = orch.agents.match_(t, raw) {
        return Some(i);
    }
    if let Some(i) = translate_intents(&o, t) {
        return Some(i);
    }
    if let Some(i) = home_intents(&o, t) {
        return Some(i);
    }
    if let Some(i) = light_intents(&o, t) {
        return Some(i);
    }
    if let Some(i) = schedule_intents(&o, t, raw) {
        return Some(i);
    }
    if let Some(i) = reminder_intents(&o, t, raw) {
        return Some(i);
    }
    if let Some(i) = project_intents(&o, t) {
        return Some(i);
    }
    if let Some(i) = orch.mails.match_(t) {
        return Some(i);
    }
    if let Some(i) = mail_filter_intents(&o, t, raw) {
        return Some(i);
    }
    if let Some(i) = mail_intents(&o, t) {
        return Some(i);
    }
    if let Some(i) = agenda_intents(&o, t, raw) {
        return Some(i);
    }
    if let Some(i) = info_intents(&o, t, raw) {
        return Some(i);
    }
    if let Some(i) = super::companion::Companion::intent(&o, t) {
        return Some(i);
    }
    if let Some(i) = windows_intents(&o, t, raw) {
        return Some(i);
    }
    if let Some(i) = orch.savoir.match_(t, raw) {
        return Some(i);
    }
    if let Some(i) = document_intents(&o, t, raw) {
        return Some(i);
    }
    if let Some(i) = orch.vision.match_(t, raw) {
        return Some(i);
    }
    if let Some(i) = orch.research.match_(t, raw) {
        return Some(i);
    }
    if let Some(i) = orch.shopping.match_(t, raw) {
        return Some(i);
    }
    web_and_apps(&o, t, raw)
}

// --- Mails -------------------------------------------------------------------------------------------

lazy_re!(mail_search_re1, format!("^(?:cherche|trouve|recherche)(?: moi)? (?:le |les |un |des |mes )?{MAIL} (?:de|du|des|sur|concernant|a propos de|avec|qui parlent? de) (.+)$"));
lazy_re!(mail_search_re2, format!("^(?:(?:ai je|j ai|est ce que j ai) (?:recu )?(?:des |un )?|(?:les |mes )?){MAIL} (?:de|du|venant de) (.+)$"));
lazy_re!(mail_count_re, format!("^(?:lis|lire|donne|donnes|dis|montre|affiche|resume|liste)(?: moi)? (?:mes |les )?(\\d{{1,2}}|un|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|quinze|vingt) (?:derniers? |nouveaux? |premiers? )?{MAIL}"));
lazy_re!(mail_latest_re1, format!("^(?:lis|lire|donne|donnes|dis|montre|affiche|resume|liste)(?: moi)? (?:mon |mes |le |les |mon dernier |mes derniers |le dernier |les derniers )?(?:derniers? |nouveaux? )?{MAIL}"));
lazy_re!(mail_latest_re2, format!(r"\b(?:ai je|j ai|est ce que j ai|y a t il) (?:recu )?(?:des |de |un )?(?:nouveaux? )?{MAIL}\b"));
lazy_re!(mail_latest_re3, format!(r"\b(?:quels sont|c est quoi) (?:mes|les) (?:derniers? )?{MAIL}\b"));
lazy_re!(mail_latest_re4, format!("^(?:mes )?(?:derniers? )?{MAIL}$"));

fn mail_intents(o: &OrchRef, t: &str) -> Option<Intent> {
    if let Some(m) = mail_search_re1().captures(t).or_else(|| mail_search_re2().captures(t)) {
        let query = m[1].trim().to_string();
        return Some(it(o, "mail_search", "Recherche mails", &query.clone(), move |o| {
            let query = query.clone();
            async move {
                let oc = o.self_ref();
                let r = tool(&o, "mcp__google__mail_chercher", json!({ "requete": query, "limite": 5 }), move |p| async move { oc.up().google.mail_chercher(&p).await }).await;
                if let Some(x) = refused(&r) {
                    return x;
                }
                if !js::truthy(&r) || r["ok"] == json!(false) {
                    return json!({ "say": if js::truthy(&r["message"]) { js::gs(&r, "message") } else { "Je n'arrive pas à consulter tes mails.".into() }, "display": "Mails indisponibles" });
                }
                let mails = js::arr(&r["mails"]);
                let Some(first) = mails.first() else { return json!({ "say": format!("Aucun mail trouvé pour {query}."), "display": "0 mail" }) };
                let n = mails.len();
                json!({ "say": format!("{n} mail{} trouvé{}. Le plus récent vient de {} : {}.", p_plural(n), p_plural(n), sender_name(&js::gs(first, "from")), js::gs(first, "subject")), "display": format!("{n} mails") })
            }
        }));
    }
    let count = mail_count_re().captures(t).map(|m| m[1].to_string());
    if count.is_some()
        || mail_latest_re1().is_match(t)
        || mail_latest_re2().is_match(t)
        || mail_latest_re3().is_match(t)
        || crate::re!(r"\b(?:dernier|derniers) (?:mail|metre|message|email) (?:que j ai recu|recu)\b").is_match(t)
        || mail_latest_re4().is_match(t)
    {
        let single = crate::re!(r"\b(?:mon|le) dernier\b").is_match(t);
        let wanted = count.and_then(|c| small_num(&c)).map(|n| n.min(20)).unwrap_or(5);
        let with_read = crate::re!(r"\b(?:tous (?:mes|les) |toutes? (?:mes|les) )(?:mails?|emails?|messages?|courriels?)\b|\b(?:y compris|meme) (?:les )?(?:deja )?lus?\b|\blus et non lus\b|\bnon lus et lus\b|\b(?:mails?|emails?|messages?) (?:deja )?lus\b").is_match(t);
        return Some(it(o, "mail_latest", if with_read { "Derniers mails" } else { "Mails non lus" }, "Gmail", move |o| async move {
            let mut params = json!({ "limite": wanted.max(1) });
            if with_read {
                params["tous"] = json!(true);
            }
            let oc = o.self_ref();
            let r = tool(&o, "mcp__google__mail_derniers", params, move |p| async move { oc.up().google.mail_derniers(&p).await }).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            if !js::truthy(&r) || r["ok"] == json!(false) {
                return json!({ "say": if js::truthy(&r["message"]) { js::gs(&r, "message") } else { "Je n'arrive pas à consulter tes mails.".into() }, "display": "Mails indisponibles" });
            }
            let mails = js::arr(&r["mails"]);
            let Some(first) = mails.first() else {
                return if with_read { json!({ "say": "Ta boîte de réception est vide.", "display": "0 mail" }) } else { json!({ "say": "Tu n'as aucun mail non lu.", "display": "0 mail non lu" }) };
            };
            let (from, subject) = (sender_name(&js::gs(first, "from")), js::gs(first, "subject"));
            if single {
                return json!({ "say": if with_read { format!("Ton dernier mail vient de {from} : {subject}.") } else { format!("Ton dernier mail non lu vient de {from} : {subject}.") }, "display": subject });
            }
            let next: Vec<String> = mails.iter().skip(1).take(2).map(|m| format!("{} : {}", sender_name(&js::gs(m, "from")), js::gs(m, "subject"))).collect();
            let n = mails.len();
            json!({
                "say": format!("{}Le plus récent vient de {from} : {subject}.{}", if with_read { String::new() } else { format!("{n} mail{} non lu{}. ", p_plural(n), p_plural(n)) }, if next.is_empty() { String::new() } else { format!(" Puis {}.", next.join(". Puis ")) }),
                "display": if with_read { format!("{n} derniers mails") } else { format!("{n} mail{} non lu{}", p_plural(n), p_plural(n)) }
            })
        }));
    }
    None
}

// --- Agenda ------------------------------------------------------------------------------------------

fn spoken_hour(h: &str) -> String {
    let s = h.replacen(':', " heures ", 1);
    crate::re!(r" heures 00$").replace(&s, " heures").into_owned()
}

fn agenda_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    if let Some(m) = crate::re!(r"^(?:ajoute|cree|note|planifie|programme|inscris)(?: moi)? (?:un |une )?(?:rendez vous|rdv|reunion|evenement|rendez vous dans (?:mon|l) agenda)\s+(.+)$").captures(t) {
        let rest = m[1].to_string();
        let day = skills::parse_day(&rest);
        let time = skills::parse_time(&rest);
        let raw_rest = crate::re!(r"(?i)^.*?(?:rendez-vous|rendez vous|rdv|réunion|reunion|événement|evenement)\s*").replace(raw, "");
        let title = crate::re!(r"(?i)\b(?:aujourd'hui|après-demain|apres-demain|demain|ce soir|ce matin|(?:lundi|mardi|mercredi|jeudi|vendredi|samedi|dimanche)(?: prochain)?|le \d{1,2}(?:er)?(?: \p{L}+)?|\d{1,2}/\d{1,2})\b").replace_all(&raw_rest, " ");
        let title = crate::re!(r"(?i)\b(?:à|a|vers)?\s*\d{1,2}\s*(?:h|heures?|:)\s*\d{0,2}\b").replace_all(&title, " ");
        let title = crate::re!(r"(?i)(?:^|\s)(?:à|a)\s+(?:midi|minuit)(?:\s|$)").replace_all(&title, " ");
        let title = crate::re!(r"(?i)^[\s,:-]*(?:pour|intitulé|appelé)?\s*").replace(&title, "");
        let title = crate::re!(r"[\s,.!?]+$").replace(&title, "");
        let title = crate::re!(r"(?i)(?:\s+(?:à|a|vers|le|pour|de))+$").replace(&title, "");
        let title = crate::re!(r"\s+").replace_all(&title, " ").trim().to_string();
        let title = if title.is_empty() { "Rendez-vous".to_string() } else { title };
        let date = day.as_ref().or(skills::parse_day("aujourd hui").as_ref()).map(|d| d.iso.clone()).unwrap_or_default();
        let label = day.as_ref().map(|d| d.label.clone()).unwrap_or_else(|| "aujourd'hui".into());
        let heure = time.unwrap_or_else(|| "09:00".into());
        return Some(it(o, "agenda_create", "Nouveau rendez-vous", &title.clone(), move |o| {
            let (title, date, label, heure) = (title.clone(), date.clone(), label.clone(), heure.clone());
            async move {
                let params = json!({ "titre": title, "date": date, "heure": heure });
                let oc = o.self_ref();
                let r = tool(&o, "mcp__google__agenda_creer", params, move |p| async move { sysr(oc.up().google.agenda_creer(&p).await) }).await;
                if let Some(x) = refused(&r) {
                    return x;
                }
                if !js::truthy(&r) || r["ok"] == json!(false) || js::truthy(&r["error"]) {
                    return json!({ "say": "Je n'ai pas pu ajouter ce rendez-vous.", "display": "Échec" });
                }
                json!({ "say": format!("C'est noté : {title}, {label} à {}.", spoken_hour(&heure)), "display": format!("{title} · {date} {heure}") })
            }
        }));
    }
    let vague = crate::re!(r"^(?:qu est ce que j ai|j ai quoi|qu ai je)(?: de prevu| au programme| a faire)?\b").is_match(t) && (skills::parse_day(t).is_some() || crate::re!(r"\b(?:de prevu|au programme|cette semaine|a faire)\b").is_match(t));
    if (vague || crate::re!(r"\b(?:quel est mon programme|mon programme|mon agenda|mes rendez vous|mon planning|mes rdv|qu est ce qui est prevu|ai je (?:des |un )?rendez vous|mes evenements|mon emploi du temps)\b").is_match(t)) && !crate::re!(r"\b(?:mail|fichier|dossier)\b").is_match(t) {
        let week = crate::re!(r"\b(?:semaine|7 prochains jours|sept prochains jours|jours a venir)\b").is_match(t);
        let day = if week { None } else { skills::parse_day(t).or_else(|| skills::parse_day("aujourd hui")) };
        let (iso, label) = day.map(|d| (d.iso, d.label)).unwrap_or_default();
        let detail = if week { "7 jours".to_string() } else { label.clone() };
        return Some(it(o, if week { "agenda_week" } else { "agenda_day" }, "Agenda", &detail, move |o| {
            let (iso, label) = (iso.clone(), label.clone());
            async move {
                let oc = o.self_ref();
                let r = if week {
                    tool(&o, "mcp__google__agenda_semaine", json!({}), move |p| async move { oc.up().google.agenda_semaine(&p).await }).await
                } else {
                    tool(&o, "mcp__google__agenda_jour", json!({ "date": iso }), move |p| async move { oc.up().google.agenda_jour(&p).await }).await
                };
                if let Some(x) = refused(&r) {
                    return x;
                }
                if !js::truthy(&r) || r["ok"] == json!(false) {
                    return json!({ "say": "Je n'arrive pas à consulter l'agenda.", "display": "Agenda indisponible" });
                }
                let events = js::arr(&r["evenements"]);
                let when = if week { "cette semaine".to_string() } else { label };
                if events.is_empty() {
                    return json!({ "say": format!("Rien de prévu {when}."), "display": format!("Agenda vide · {when}") });
                }
                let describe = |e: &J| {
                    let mut hour = js::gs(e, "heure");
                    if hour.is_empty() {
                        if let Some(c) = crate::re!(r"T(\d{2}):(\d{2})").captures(&js::gs(e, "debut")) {
                            hour = format!("{}:{}", &c[1], &c[2]);
                        }
                    }
                    format!("{}{}", if hour.is_empty() { String::new() } else { format!("à {}, ", spoken_hour(&hour)) }, js::gs(e, "titre"))
                };
                let list: Vec<String> = events.iter().take(5).map(describe).collect();
                json!({ "say": format!("{} rendez-vous {when} : {}.", events.len(), list.join(" ; ")), "display": format!("{} rendez-vous · {when}", events.len()) })
            }
        }));
    }
    None
}

// --- Météo, calcul, conversions ------------------------------------------------------------------------

fn info_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    if crate::re!(r"\b(?:meteo|quel temps|temps qu il fait|temps fait il|temps fera t il|temps va t il faire|va t il pleuvoir|il va pleuvoir|pleut il|il pleut|fait il (?:beau|chaud|froid)|temperature (?:dehors|exterieure)|combien de degres|prevision)\b").is_match(t)
        && !crate::re!(r"\b(?:temps de (?:trajet|cuisson|reponse)|combien de temps)\b").is_match(t)
    {
        let day = skills::parse_day(t);
        let spoken = super::skill_intents::city_in_raw(raw);
        let detail = spoken.clone().unwrap_or_else(|| "Open-Meteo".into());
        return Some(it(o, "weather", "Météo", &detail, move |o| {
            let (day, spoken) = (day.clone(), spoken.clone());
            async move {
                let Some(city) = spoken.clone().or_else(|| super::skill_intents::default_city(&o)) else { return json!({ "say": "Pour quelle ville ? Dis par exemple : météo à Lyon.", "display": "Ville inconnue" }) };
                match skills::weather(&city, day.as_ref()).await {
                    Ok(w) => {
                        if js::gb(&w, "ok") && spoken.is_some() {
                            o.status.d.borrow_mut().weather_city = Some(js::gs(&w["place"], "name"));
                            o.status.save();
                        }
                        o.record("meteo_locale", json!({ "ville": city, "jour": day.as_ref().map(|d| d.iso.clone()).unwrap_or_else(|| "maintenant".into()), "reponse": w["say"] }));
                        json!({ "say": w["say"], "display": if js::truthy(&w["display"]) { w["display"].clone() } else { json!("Météo") } })
                    }
                    Err(_) => json!({ "say": "Le service météo ne répond pas pour l'instant.", "display": "Météo indisponible" }),
                }
            }
        }));
    }
    if let Some(c) = skills::parse_conversion(t) {
        let text = js::gs(&c, "text");
        return Some(Intent::now("conversion", "Conversion", &text, json!({ "say": text, "display": text })));
    }
    if let Some(c) = skills::parse_calculation(t) {
        let text = js::gs(&c, "text");
        return Some(Intent::now("calculation", "Calcul", &format!("{} = {text}", js::gs(&c, "expression")), json!({ "say": format!("Ça fait {}.", text.replacen('-', "moins ", 1)), "display": format!("= {text}") })));
    }
    None
}

// --- Commandes Windows -------------------------------------------------------------------------------

fn windows_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    let orch = o.up();
    if let Some(m) = crate::re!(r"^(eteins|arrete|redemarre|relance)(?: moi)? (?:l ordinateur|l ordi|le pc|la machine|windows|le portable)$").captures(t) {
        let action = if matches!(&m[1], "eteins" | "arrete") { "eteindre" } else { "redemarrer" };
        return Some(it(o, &format!("power_{action}"), if action == "eteindre" { "Extinction" } else { "Redémarrage" }, "dans 30 secondes", move |o| async move {
            let r = sys_tool(&o, "mcp__system__alimentation", "alimentation", json!({ "action": action })).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            if js::gb(&r, "ok") {
                json!({ "say": format!("{} l'ordinateur dans 30 secondes. Dis « annule l'extinction » pour l'arrêter.", if action == "eteindre" { "J'éteins" } else { "Je redémarre" }), "display": r["message"] })
            } else {
                json!({ "say": "Je n'ai pas pu le faire.", "display": "Échec" })
            }
        }));
    }
    if crate::re!(r"^(?:annule|stoppe|arrete) (?:l extinction|l arret|le redemarrage|l arret de l ordinateur)$|^n eteins pas(?: l ordinateur)?$").is_match(t) {
        return Some(it(o, "power_cancel", "Extinction annulée", "shutdown /a", |o| async move {
            let r = tool(&o, "mcp__system__alimentation_annuler", json!({ "action": "annuler" }), |_| async move { sysr(super::system::alimentation(&json!({ "action": "annuler" })).await) }).await;
            refused(&r).unwrap_or(json!({ "say": "Extinction annulée.", "display": "Annulée" }))
        }));
    }
    if crate::re!(r"^(?:vide|videz|nettoie)(?: moi)? (?:la |ma )?corbeille$").is_match(t) {
        return Some(it(o, "recycle_bin", "Corbeille", "vidage", |o| async move {
            let r = sys_tool(&o, "mcp__system__vider_corbeille", "vider_corbeille", json!({})).await;
            refused(&r).unwrap_or(json!({ "say": if js::gb(&r, "ok") { "Corbeille vidée." } else { "Je n'ai pas pu vider la corbeille." }, "display": "Corbeille" }))
        }));
    }
    if let Some(m) = crate::re!(r"^(baisse|diminue|reduis|monte|augmente|hausse)(?: un peu)? (?:la )?luminosite(?: de l ecran)?(?: un peu)?$|^(?:mets |rends )?l ecran plus (clair|lumineux|sombre)$").captures(t) {
        let down = m.get(1).map(|x| matches!(x.as_str(), "baisse" | "diminue" | "reduis")).unwrap_or(false) || m.get(2).map(|x| x.as_str() == "sombre").unwrap_or(false);
        let ecart = (if t.contains("un peu") { 10 } else { 20 }) * (if down { -1 } else { 1 });
        return Some(it(o, "brightness", "Luminosité", &format!("{}{ecart} %", if ecart > 0 { "+" } else { "" }), move |o| async move {
            let r = sys_tool(&o, "mcp__system__luminosite", "luminosite", json!({ "ecart": ecart })).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            json!({
                "say": if js::gb(&r, "ok") { format!("Luminosité à {} pour cent.", js::s(&r["valeur"])) } else if js::truthy(&r["message"]) { js::gs(&r, "message") } else { "Luminosité non réglable.".into() },
                "display": if js::truthy(&r["valeur"]) { format!("{}%", js::s(&r["valeur"])) } else { "Luminosité".into() }
            })
        }));
    }
    const ART: &str = "(?:le |la |les |l |mon |ma |mes )?";
    let re = |p: String| regex::Regex::new(&p).unwrap();
    let settings = re(format!("^(?:ouvre|affiche|montre|va dans)(?: moi)? {ART}(?:page (?:des |de )|menu (?:des |de ))?(?:parametres|reglages)(?: (?:de |du |des |d )?{ART}(.+))?$")).captures(t);
    let toggle = re(format!("^(active|desactive|allume|eteins|coupe|mets|enleve|retire)(?: moi)? {ART}(bluetooth|wifi|wi fi|mode avion|mode sombre|mode nuit|veilleuse|economiseur de batterie)$")).captures(t);
    let is_toggle = toggle.is_some();
    let page_name = settings.as_ref().and_then(|m| m.get(1).map(|x| x.as_str().trim().to_string())).or_else(|| toggle.as_ref().map(|m| m[2].to_string())).unwrap_or_default();
    if let Some((page, label)) = (!page_name.is_empty()).then(|| settings_page(&page_name)).flatten() {
        return Some(it(o, "settings", "Paramètres Windows", label, move |o| async move {
            let r = sys_tool(&o, "mcp__system__ouvrir_parametres", "ouvrir_parametres", json!({ "page": page })).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            let say = if is_toggle {
                format!("Je t'ouvre les réglages : {label} se bascule d'un clic, Windows ne me laisse pas le faire seul.")
            } else if label == "les paramètres" {
                "J'ouvre les paramètres de Windows.".to_string()
            } else {
                format!("J'ouvre les réglages : {label}.")
            };
            json!({ "say": if js::gb(&r, "ok") { say } else { "Je n'ai pas pu ouvrir les paramètres.".into() }, "display": label })
        }));
    }
    if let Some(m) = re(format!("^(?:ouvre|lance|demarre|affiche|montre)(?: moi)? {ART}(.+)$")).captures(t) {
        if let Some((exe, label)) = windows_app(m[1].trim()) {
            let mut i = it(o, "open_app", &format!("Lancement {label}"), exe, move |o| async move {
                let r = sys_tool(&o, "mcp__system__ouvrir_application", "ouvrir_application", json!({ "nom": exe })).await;
                refused(&r).unwrap_or(json!({ "say": if js::truthy(&r["error"]) { format!("Je n'arrive pas à ouvrir {label}.") } else { format!("J'ouvre {label}.") }, "display": label }))
            });
            i.detail = exe.into();
            return Some(i);
        }
    }
    if crate::re!(r"^(?:verrouille|bloque)(?: l ordinateur| le pc| la session| l ecran| windows| la machine)?$").is_match(t) {
        return Some(it(o, "lock", "Verrouillage", "session Windows", |o| async move {
            let r = sys_tool(&o, "mcp__system__verrouiller", "verrouiller", json!({})).await;
            refused(&r).unwrap_or(json!({ "say": null, "display": "Session verrouillée" }))
        }));
    }
    if crate::re!(r"^(?:(?:mets|passe) (?:l ordinateur |le pc |la machine )?en veille|mise en veille)$").is_match(t) {
        return Some(it(o, "sleep", "Mise en veille", "ordinateur", |o| async move {
            let r = sys_tool(&o, "mcp__system__veille", "veille", json!({})).await;
            refused(&r).unwrap_or(json!({ "say": null, "display": "Mise en veille" }))
        }));
    }
    if let Some(m) = crate::re!(r"\bluminosite (?:de l ecran )?(?:a |au |sur )?(\d{1,3})\b").captures(t).or_else(|| crate::re!(r"^(?:mets|regle|passe) (?:la )?luminosite (?:de l ecran )?(?:a |au |sur )?(\d{1,3})").captures(t)) {
        let pct = m[1].parse::<i64>().unwrap_or(50).clamp(10, 100);
        return Some(it(o, "brightness", "Luminosité", &format!("{pct} %"), move |o| async move {
            let r = sys_tool(&o, "mcp__system__luminosite", "luminosite", json!({ "pourcentage": pct })).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            json!({ "say": if r["ok"] == json!(false) { r["message"].clone() } else { json!(format!("Luminosité à {pct} pour cent.")) }, "display": format!("{pct}%") })
        }));
    }
    if crate::re!(r"^(?:quelles? (?:sont )?(?:les |mes )?(?:applis?|applications?|logiciels?|programmes?)(?: qui)? (?:sont |j ai )?installee?s?|(?:liste|montre|affiche|donne)(?: moi)? (?:les |mes |toutes les |tous les |toutes mes |tous mes )?(?:applis?|applications?|logiciels?|programmes?) installee?s?|(?:mes |les )?(?:applis?|applications?|logiciels?|programmes?) installee?s?)(?: sur (?:mon |le |l )?(?:pc|ordi|ordinateur))?$").is_match(t) {
        return Some(it(o, "apps_installed", "Applications", "installées", |o| async move {
            let card = o.list_cards.apps_card("");
            let (count, running) = (js::s(&card["count"]), js::gn_or(&card, "running", 0.0) as i64);
            json!({
                "say": format!("{count} applications installées{}. La liste numérotée est à l'écran : dis « lance l'application » suivi du numéro.", if running > 0 { format!(", dont {running} ouvertes en ce moment") } else { String::new() }),
                "display": format!("{count} applications")
            })
        }));
    }
    if let Some(m) = crate::re!(r"^(?:lance|ouvre|demarre)(?: moi)? l ?(?:application|appli) (?:numero |n )?(\d{1,3})$").captures(t) {
        let n: i64 = m[1].parse().unwrap_or(0);
        return Some(it(o, "apps_launch_number", "Application", &format!("n° {n}"), move |o| async move {
            let Some(name) = o.list_cards.by_number("appli", n) else { return json!({ "say": format!("Il n'y a pas d'application {n}. Dis « liste mes applications installées »."), "display": "Introuvable" }) };
            let r = o.list_cards.action("appli", &name, "lancer").await;
            json!({ "say": r["say"], "display": name })
        }));
    }
    if crate::re!(r"\b(?:quelles (?:sont les )?(?:fenetres|applications|applis)(?: ouvertes)?|liste (?:les )?(?:fenetres|applications ouvertes)|(?:fenetres|applications) ouvertes|qu est ce qui est ouvert)\b").is_match(t) {
        return Some(it(o, "windows_list", "Fenêtres", "liste", |o| async move {
            let ds = &o.desktop_state;
            if ds.ready() {
                o.show_widget("tool", super::desktop_intents::desktop_card(&o, None, false));
                let mut apps: Vec<String> = vec![];
                for w in ds.windows() {
                    let a = js::gs(&w, "app");
                    if !apps.contains(&a) {
                        apps.push(a);
                    }
                }
                let n = apps.len();
                return json!({ "say": format!("{n} application{} ouverte{} : {}. La liste est à l'écran.", p_plural(n), p_plural(n), apps.iter().take(8).cloned().collect::<Vec<_>>().join(", ")), "display": apps.iter().take(6).cloned().collect::<Vec<_>>().join(", ") });
            }
            let r = sys_tool(&o, "mcp__system__lister_fenetres", "lister_fenetres", json!({})).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            let list = js::arr(&r);
            let mut names: Vec<String> = vec![];
            for w in &list {
                let n = js::gs(w, "ProcessName");
                if !n.is_empty() && !crate::re!(r"(?i)^(?:electron|applicationframehost|textinputhost|systemsettings)$").is_match(&n) && !names.contains(&n) {
                    names.push(n);
                }
            }
            if names.is_empty() {
                return json!({ "say": "Aucune fenêtre ouverte.", "display": "0 fenêtre" });
            }
            let rows: Vec<J> = list.iter().enumerate().map(|(i, w)| json!({ "icon": "▣", "title": format!("{}. {}", i + 1, if js::truthy(&w["Application"]) { js::gs(w, "Application") } else { js::gs(w, "ProcessName") }), "sub": js::gs(w, "MainWindowTitle") })).collect();
            o.show_widget("tool", json!({ "title": "Fenêtres ouvertes", "tool": "fenetres", "count": list.len(), "ttl": 120000, "rows": rows, "footer": "« bascule sur la fenêtre <titre> »" }));
            json!({ "say": format!("{} applications ouvertes : {}.", names.len(), names.iter().take(10).cloned().collect::<Vec<_>>().join(", ")), "display": names.iter().take(6).cloned().collect::<Vec<_>>().join(", ") })
        }));
    }
    if let Some(m) = crate::re!(r"^(?:bascule|passe|va|reviens) (?:sur |a |dans |vers )?la fenetre (?:de |d )?(.+)$").captures(t).or_else(|| crate::re!(r"^(?:affiche|montre)(?: moi)? la fenetre (?:de |d )?(.+)$").captures(t)) {
        let title = crate::re!(r"(?i)^.*fen[êe]tre\s+(?:de\s+|d['’]\s*)?").replace(raw, "");
        let title = crate::re!(r"[.!?]+$").replace(&title, "").trim().to_string();
        let title = if title.is_empty() { m[1].to_string() } else { title };
        return Some(it(o, "window_focus", "Fenêtre", &title.clone(), move |o| {
            let title = title.clone();
            async move {
                let r = sys_tool(&o, "mcp__system__activer_fenetre", "activer_fenetre", json!({ "titre": title })).await;
                refused(&r).unwrap_or(json!({ "say": if js::gb(&r, "ok") { Value::Null } else { json!(format!("Je ne trouve pas de fenêtre {title}.")) }, "display": title }))
            }
        }));
    }
    if crate::re!(r"\b(?:lis|lire|que contient|qu y a t il dans|qu est ce qu il y a dans) (?:le |mon )?presse papier\b").is_match(t) {
        return Some(it(o, "clipboard_read", "Presse-papier", "lecture", |o| async move {
            let r = sys_tool(&o, "mcp__system__presse_papier_lire", "presse_papier_lire", json!({})).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            let txt = js::gs(&r, "texte").trim().to_string();
            json!({ "say": if txt.is_empty() { "Le presse-papier est vide.".to_string() } else { format!("Le presse-papier contient : {}", js::slice_chars(&txt, 400)) }, "display": if txt.is_empty() { "Vide".to_string() } else { js::slice_chars(&txt, 40) } })
        }));
    }
    if let Some(m) = crate::re!(r"^(?:ferme|quitte)(?: l application| le logiciel| la fenetre de)? ([a-z0-9+.-]+(?: [a-z0-9+.-]+)?)$").captures(t) {
        if !crate::re!(r"^(?:la|le|les|tout|toutes?|ca|cette|ce)$").is_match(&m[1]) {
            let spoken = m[1].trim().to_string();
            let proc_name = super::system::app_registry(&spoken).map(String::from).unwrap_or_else(|| spoken.replace(' ', ""));
            let name = js::capitalize(&spoken);
            let _ = &orch;
            return Some(it(o, "app_close", "Fermeture", &name.clone(), move |o| {
                let (proc_name, name) = (proc_name.clone(), name.clone());
                async move {
                    let r = sys_tool(&o, "mcp__system__fermer_application", "fermer_application", json!({ "nom": proc_name })).await;
                    if let Some(x) = refused(&r) {
                        return x;
                    }
                    let failed = js::truthy(&r["error"]) || r["ok"] == json!(false);
                    json!({
                        "say": if failed { format!("Je n'ai pas pu fermer {name}{}.", if js::truthy(&r["error"]) { format!(" : {}", js::gs(&r, "error")) } else { String::new() }) } else { format!("{name} est fermé.") },
                        "display": name
                    })
                }
            }));
        }
    }
    None
}

// --- Documents : Google Docs, dictée, correction -----------------------------------------------------

fn document_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    if crate::re!(r"^(?:cree|ouvre|lance|fais|demarre|commence)(?: moi)? (?:un |une )(?:nouveau |nouvelle )?(?:google doc|google docs|document google(?: docs)?|doc google|document docs)$").is_match(t) || crate::re!(r"^(?:nouveau|nouvelle) (?:google doc|document google)$").is_match(t) {
        return Some(it(o, "gdoc_new", "Nouveau Google Doc", "docs.new", |o| async move {
            let r = sys_tool(&o, "mcp__system__ouvrir_url", "ouvrir_url", json!({ "url": "https://docs.new" })).await;
            if let Some(x) = refused(&r) {
                return x;
            }
            o.start_dictation("Google Docs");
            json!({ "say": "Nouveau document ouvert. Dicte ton texte, je l'écris. Dis « corrige le texte » pour la correction, ou « fin de la dictée ».", "display": "Google Docs · dictée" })
        }));
    }
    if crate::re!(r"^(?:commence|lance|demarre|active|passe en)(?: la| le mode| mode)? dictee$|^mode dictee$|^dictee$|^je vais (?:te )?dicter\b").is_match(t) {
        return Some(it(o, "dictation_start", "Dictée", "document au premier plan", |o| async move {
            o.start_dictation("document");
            json!({ "say": "Je t'écoute : chaque phrase sera écrite dans le document ouvert. Dis « fin de la dictée » pour arrêter.", "display": "Dictée active" })
        }));
    }
    if let Some(m) = crate::re!(r"(?i)^\s*(?:jarvis[, ]+)?(?:tape|saisis|écris dans le (?:document|doc)|ecris dans le (?:document|doc)|écris\s*:|ecris\s*:)\s*:?\s*(.+)$").captures(raw) {
        let content = m[1].trim().to_string();
        return Some(it(o, "type_text", "Saisie", &js::slice_chars(&content, 30), move |o| {
            let content = content.clone();
            async move {
                let say = o.dictate(&content).await;
                let n = content.chars().count();
                json!({ "say": say, "display": if say.is_some() { "Saisie impossible".to_string() } else { format!("{n} caractères") } })
            }
        }));
    }
    if let Some(m) = crate::re!(r"^corrige(?: moi)? (le texte|le document|l orthographe|les fautes|mon texte|ce texte|la selection|le doc)$").captures(t) {
        let whole = &m[1] != "la selection";
        return Some(it(o, "text_correct", "Correction", "Grammalecte", move |o| async move {
            let say = o.correct_document(whole).await;
            json!({ "say": say, "display": "Correction" })
        }));
    }
    None
}

// --- Liens et chemins, web, applications du menu Démarrer ---------------------------------------------

fn web_and_apps(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    if let Some(m) = crate::re!(r"(?i)^(?:jarvis[,\s]+)?(?:ouvre|affiche)\s+(https?://\S+|[a-zA-Z]:\\[^\n]+?)\s*[.!]?$").captures(raw.trim()) {
        let target = m[1].to_string();
        if crate::re!(r"(?i)^https?:").is_match(&target) {
            return Some(it(o, "web", "Page web", &target.clone(), move |o| {
                let target = target.clone();
                async move {
                    let r = sys_tool(&o, "mcp__system__ouvrir_url", "ouvrir_url", json!({ "url": target })).await;
                    refused(&r).unwrap_or(json!({ "say": "J'ouvre la page.", "display": target }))
                }
            }));
        }
        let name = basename(&target);
        return Some(it(o, "file_open", "Ouvrir un fichier", &name.clone(), move |o| {
            let (target, name) = (target.clone(), name.clone());
            async move {
                let (oc, t2) = (o.self_ref(), target.clone());
                let r = tool(&o, "mcp__system__ouvrir_fichier", json!({ "chemin": target }), move |_| async move { sysr(oc.up().file_search.open(&t2)) }).await;
                refused(&r).unwrap_or(json!({ "say": format!("J'ouvre {name}."), "display": name }))
            }
        }));
    }
    if let Some(w) = skills::parse_web(t, raw) {
        let label = js::gs(&w, "label");
        let detail = if js::truthy(&w["query"]) { js::gs(&w, "query") } else { js::gs(&w, "url") };
        return Some(it(o, "web", &label, &detail, move |o| {
            let w = w.clone();
            async move {
                let mut params = json!({ "url": w["url"] });
                if js::truthy(&w["browser"]) {
                    params["navigateur"] = w["browser"].clone();
                }
                let r = sys_tool(&o, "mcp__system__ouvrir_url", "ouvrir_url", params).await;
                refused(&r).unwrap_or(json!({ "say": if js::truthy(&r["error"]) { json!("Je n'arrive pas à ouvrir le navigateur.") } else { w["say"].clone() }, "display": w["url"] }))
            }
        }));
    }
    let m = crate::re!(r"^(?:ouvre|lance|demarre|execute)(?: moi)? (?:l application |le logiciel |le programme )?(.+)$").captures(t)?;
    if super::system::app_registry(m[1].trim()).is_some() {
        return None;
    }
    let orch = o.up();
    let (found, _) = orch.app_index.find(&m[1])?;
    let name = found.name.clone();
    Some(it(o, "open_app", &format!("Lancement {name}"), "menu Démarrer", move |o| {
        let found = found.clone();
        async move {
            let name = found.name.clone();
            let (oc, f2) = (o.self_ref(), found.clone());
            let r = tool(&o, "mcp__system__ouvrir_application", json!({ "nom": name }), move |_| async move { sysr(oc.up().app_index.open(&f2)) }).await;
            refused(&r).unwrap_or(json!({ "say": if js::truthy(&r["error"]) { format!("Je n'arrive pas à lancer {name}.") } else { format!("J'ouvre {name}.") }, "display": name }))
        }
    }))
}

#[cfg(test)]
mod tests_mails_polis {
    use super::*;

    #[test]
    fn une_demande_polie_de_derniers_mails_est_reconnue_apres_normalisation() {
        let brut = crate::orch::answer_cache::fold("Jarvis, si tu peux me donner les derniers mails que j'ai reçus.");
        let t = crate::orch::prefilter::normalize_request(&brut);
        assert!(mail_latest_re1().is_match(&t), "« {t} » devrait être une demande de derniers mails");
    }

    #[test]
    fn ouverture_cards_et_modules_par_nom_ou_numero_sans_ia() {
        assert_eq!(crate::modules::number_default_slot(120).as_deref(), Some("nomade_view"));
        assert_eq!(crate::modules::number_default_slot(106).as_deref(), Some("tool:Mes listes"));
        let weak = std::rc::Weak::<crate::orch::Orchestrator>::new();
        for phrase in [
            "ouvre le module Nomad.",
            "ouvre la card Nomade",
            "ouvre la carte nomade",
            "ouvre moi la card nomade 120",
        ] {
            let folded = crate::orch::prefilter::trim_end(&crate::orch::answer_cache::fold(phrase));
            let m = crate::orch::nomade::intents(&weak, &folded);
            assert_eq!(m.as_ref().map(|i| i.kind.as_str()), Some("nomade_show"), "phrase non reconnue : {phrase}");
        }
    }
}
