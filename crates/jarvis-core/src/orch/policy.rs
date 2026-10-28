// Politique de sécurité de Jarvis (orchestrator/policy.js) : ce qui passe tout seul, et ce qui exige
// un accord. Lecture et recherche automatiques ; toute écriture ou commande système demande une
// validation (accord oral ou code à 4 chiffres).

use serde_json::Value;

use super::zones::Zones;
use crate::js::{self, J};

pub const AUTO_ALLOWED: &[&str] = &[
    "Read", "Glob", "Grep", "WebSearch", "WebFetch", "TodoWrite", "NotebookRead", "BashOutput",
    "mcp__jarvis__memoire_lire", "mcp__jarvis__memoire_ajouter", "mcp__jarvis__demander_precision",
    "mcp__jarvis__historique_lire", "mcp__jarvis__reunions_chercher", "mcp__jarvis__reunion_lire",
    "mcp__jarvis__listes_chercher", "mcp__jarvis__listes_lire", "mcp__jarvis__documents_chercher", "mcp__jarvis__savoir_chercher",
    "mcp__system__ouvrir_application", "mcp__system__media", "ouvrir_application", "media",
    "mcp__system__lister_fenetres", "mcp__system__activer_fenetre", "mcp__system__ouvrir_url",
    "mcp__system__ouvrir_dossier", "mcp__system__presse_papier_lire", "mcp__system__presse_papier_ecrire",
    "mcp__system__capture_ecran", "mcp__system__lire_ecran_ocr", "mcp__system__projets", "mcp__system__lumiere", "mcp__system__maison",
    "mcp__system__volume", "mcp__system__luminosite", "mcp__system__fenetre_active", "mcp__system__sourdine_contextuelle",
    "mcp__google__mail_derniers", "mcp__google__mail_chercher", "mcp__google__mail_lire", "mcp__google__agenda_jour",
    "mcp__google__agenda_semaine", "mcp__google__google_statut",
    // AGY (Antigravity)
    "view_file", "read_file", "view_file_outline", "view_code_item", "find_by_name", "grep_search", "list_dir", "LS",
    "read_url_content", "search_web", "read_resource", "list_resources", "ask_question", "call_mcp_tool", "manage_task",
];

pub const WRITE_TOOLS: &[&str] = &[
    "Write", "Edit", "MultiEdit", "NotebookEdit",
    "write_to_file", "write_file", "create_file", "edit_file", "replace_file_content", "multi_replace_file_content",
];
pub const READ_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "NotebookRead", "LS",
    "view_file", "read_file", "view_file_outline", "view_code_item", "find_by_name", "grep_search", "list_dir",
];
pub const SHELL_TOOLS: &[&str] = &["Bash", "run_command", "send_command_input"];
pub const PATH_KEYS: &[&str] = &[
    "file_path", "notebook_path", "path", "Path", "AbsolutePath", "TargetFile", "target_file",
    "DirectoryPath", "SearchDirectory", "SearchPath", "file", "File", "filename", "destination",
];

pub fn label(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "Read" => "lecture",
        "Write" => "écriture",
        "Edit" => "modification",
        "NotebookEdit" => "modification de notebook",
        "Bash" => "commande système",
        "Glob" => "recherche de fichiers",
        "Grep" => "recherche dans le code",
        "WebSearch" => "recherche web",
        "WebFetch" => "consultation web",
        "Task" => "sous-agent",
        "TodoWrite" => "plan de travail",
        "mcp__jarvis__memoire_lire" => "consultation de la mémoire",
        "mcp__jarvis__memoire_ajouter" => "mémorisation",
        "mcp__jarvis__demander_precision" => "demande de précision",
        "mcp__jarvis__memoire_oublier" => "effacement de mémoire",
        "mcp__jarvis__memoire_effacer" => "effacement complet de la mémoire",
        "mcp__jarvis__historique_lire" => "consultation du journal",
        "mcp__jarvis__reunions_chercher" => "recherche dans les réunions",
        "mcp__jarvis__reunion_lire" => "lecture d’une réunion",
        "mcp__jarvis__listes_chercher" => "recherche dans les listes",
        "mcp__jarvis__listes_lire" => "lecture des listes",
        "mcp__jarvis__documents_chercher" => "recherche dans les documents",
        "mcp__jarvis__savoir_chercher" => "recherche dans les dossiers Data",
        "mcp__system__ouvrir_application" => "lancement d'application",
        "mcp__system__fermer_application" => "fermeture d'application",
        "mcp__system__lister_fenetres" => "liste des fenêtres",
        "mcp__system__activer_fenetre" => "activation de fenêtre",
        "mcp__system__ouvrir_url" => "ouverture web",
        "mcp__system__ouvrir_dossier" => "ouverture de dossier",
        "mcp__system__presse_papier_lire" => "lecture presse-papier",
        "mcp__system__presse_papier_ecrire" => "copie dans le presse-papier",
        "mcp__system__capture_ecran" => "capture d'écran",
        "mcp__system__lire_ecran_ocr" => "lecture de l'écran (OCR)",
        "mcp__system__volume" => "réglage volume",
        "mcp__system__luminosite" => "réglage luminosité",
        "mcp__system__fenetre_active" => "fenêtre active",
        "mcp__system__etat_bureau" => "état du bureau",
        "mcp__system__fenetre" => "action sur une fenêtre",
        "mcp__system__fenetre_fermer" => "fermeture d’une fenêtre",
        "mcp__system__onglets" => "onglets de Chrome",
        "mcp__system__onglet_agir" => "action dans une page de Chrome",
        "mcp__system__ui_lister" => "boutons d’une application",
        "mcp__system__ui_agir" => "clic ou saisie dans une application",
        "mcp__system__sourdine_contextuelle" => "sourdine contextuelle",
        "mcp__system__media" => "contrôle de la musique",
        "mcp__system__verrouiller" => "verrouillage session",
        "mcp__system__veille" => "mise en veille",
        "mcp__system__alimentation" => "extinction / redémarrage",
        "mcp__system__alimentation_annuler" => "annulation de l’extinction",
        "mcp__system__vider_corbeille" => "vidage de la corbeille",
        "mcp__system__ouvrir_parametres" => "paramètres Windows",
        "mcp__system__tmux" => "tmux",
        "mcp__system__tmux_claude" => "Claude Code dans tmux",
        "mcp__system__tmux_dicter" => "message dans tmux",
        "mcp__system__tmux_shell" => "commande dans tmux",
        "mcp__system__terminal_dicter" => "texte dans le terminal",
        "mcp__system__programmation_action" => "action programmée (après 120 s de compte à rebours)",
        "mcp__system__terminal_valider" => "validation (Entrée) dans le terminal",
        "mcp__system__tmux_fermer" => "fermeture tmux",
        "mcp__google__mail_derniers" => "recherche des derniers emails",
        "mcp__google__mail_chercher" => "recherche dans les emails",
        "mcp__google__mail_lire" => "lecture d’un email",
        "mcp__google__mail_envoyer" => "envoi d'un email",
        "mcp__google__agenda_jour" => "consultation de l'agenda",
        "mcp__google__agenda_semaine" => "consultation de l'agenda de la semaine",
        "mcp__google__agenda_creer" => "création de rendez-vous",
        "mcp__google__google_statut" => "état du compte Google",
        "mcp__google__google_configurer" => "configuration du compte Google",
        "view_file" | "read_file" | "view_file_outline" | "view_code_item" => "lecture",
        "write_to_file" | "write_file" | "create_file" => "écriture",
        "replace_file_content" | "edit_file" | "MultiEdit" => "modification",
        "multi_replace_file_content" => "modification multiple",
        "run_command" | "send_command_input" => "commande système",
        "find_by_name" => "recherche de fichiers",
        "grep_search" => "recherche dans le code",
        "list_dir" | "LS" => "liste de fichiers",
        "read_url_content" => "consultation web",
        "read_resource" | "list_resources" => "ressource",
        "ask_question" => "demande de précision",
        "call_mcp_tool" => "outil MCP",
        "generate_image" => "génération image",
        "invoke_subagent" => "sous-agent",
        "manage_task" => "suivi de tâche",
        _ => return None,
    })
}

const FULL_MAX: usize = 1200;

/// Texte ramené à une ligne et coupé à `max` caractères (« … » final).
pub fn shorten(value: &str, max: usize) -> String {
    shorten_mode(value, max, false)
}

fn shorten_mode(value: &str, max: usize, full: bool) -> String {
    let text = crate::re!(r"\s+").replace_all(value, " ").trim().to_string();
    let limit = if full { FULL_MAX } else { max };
    if text.chars().count() > limit {
        format!("{}…", text.chars().take(limit.saturating_sub(1)).collect::<String>())
    } else {
        text
    }
}

/// shorten(String(x)) d'une valeur JSON.
pub fn shorten_v(v: &J, max: usize) -> String {
    shorten(&js::s(v), max)
}

struct D {
    full: bool,
}

impl D {
    fn sh(&self, v: &J, max: usize) -> String {
        shorten_mode(&js::s(v), max, self.full)
    }
    fn base(&self, v: &J) -> String {
        let p = js::s(v);
        if self.full {
            shorten_mode(&p, FULL_MAX, true)
        } else {
            p.rsplit(['\\', '/']).next().unwrap_or("").to_string()
        }
    }
}

/// Premier champ non vide (a || b || c).
fn or(input: &J, keys: &[&str]) -> J {
    for k in keys {
        let v = js::get(input, k);
        if js::truthy(v) {
            return v.clone();
        }
    }
    Value::Null
}

/// Phrase courte affichée sur la branche pendant que l'outil s'exécute.
pub fn describe(tool: &str, input: &J) -> String {
    describe_mode(tool, input, false)
}

/// Même description, sans aucune coupure : commande entière, chemin complet.
pub fn describe_full(tool: &str, input: &J) -> String {
    describe_mode(tool, input, true)
}

fn describe_mode(tool: &str, input: &J, full: bool) -> String {
    let d = D { full };
    let label = label(tool).map(String::from).unwrap_or_else(|| tool.to_string());
    let g = |k: &str| js::gs(input, k);
    match tool {
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "view_file" | "read_file" | "view_file_outline" | "view_code_item"
        | "write_to_file" | "write_file" | "create_file" | "edit_file" | "replace_file_content" | "multi_replace_file_content" => {
            let p = d.base(&Value::String(target_of(input)));
            if p.is_empty() { label } else { format!("{label} : {p}") }
        }
        "Bash" => format!("{label} : {}", d.sh(js::get(input, "command"), 50)),
        "run_command" | "send_command_input" => format!("{label} : {}", d.sh(&or(input, &["CommandLine", "command", "Input"]), 50)),
        "Glob" | "Grep" => format!("{label} : {}", d.sh(js::get(input, "pattern"), 40)),
        "find_by_name" => format!("{label} : {}", d.sh(&or(input, &["Pattern", "pattern"]), 40)),
        "grep_search" => format!("{label} : {}", d.sh(&or(input, &["Query", "query"]), 40)),
        "list_dir" | "LS" => format!("{label} : {}", d.base(&Value::String(target_of(input)))),
        "WebSearch" | "search_web" => format!("{label} : {}", d.sh(js::get(input, "query"), 40)),
        "WebFetch" | "read_url_content" => format!("{label} : {}", d.sh(&or(input, &["url", "Url"]), 40)),
        "Task" | "invoke_subagent" => {
            let mut v = or(input, &["description", "Role"]);
            if !js::truthy(&v) {
                v = input.get("Subagents").and_then(|s| s.get(0)).and_then(|s| s.get("Role")).cloned().unwrap_or(Value::Null);
            }
            format!("{label} : {}", d.sh(&v, 40))
        }
        "call_mcp_tool" => {
            let server = g("ServerName");
            let mcp_tool = g("ToolName");
            let args = js::get(input, "Arguments").clone();
            let a = |k: &str| js::gs(&args, k);
            let av = |k: &str| js::get(&args, k).clone();
            if server == "jarvis" {
                match mcp_tool.as_str() {
                    "memoire_lire" => return format!("consultation de la mémoire{}", if js::truthy(&av("recherche")) { format!(" : {}", d.sh(&av("recherche"), 30)) } else { String::new() }),
                    "memoire_ajouter" => return format!("mémorisation : {}", d.sh(&av("texte"), 30)),
                    "memoire_oublier" => return "effacement de mémoire".into(),
                    "historique_lire" => return "consultation du journal".into(),
                    "reunions_chercher" => return format!("recherche dans les réunions{}", if js::truthy(&av("recherche")) { format!(" : {}", d.sh(&av("recherche"), 30)) } else { String::new() }),
                    "reunion_lire" => return format!("lecture d’une réunion{}", if js::truthy(&av("reference")) { format!(" : {}", d.sh(&av("reference"), 30)) } else { String::new() }),
                    "listes_chercher" => return format!("recherche dans les listes{}", if js::truthy(&av("recherche")) { format!(" : {}", d.sh(&av("recherche"), 30)) } else { String::new() }),
                    "listes_lire" => return "lecture des listes".into(),
                    "savoir_chercher" => return format!("recherche dans les dossiers Data{}", if js::truthy(&av("recherche")) { format!(" : {}", d.sh(&av("recherche"), 30)) } else { String::new() }),
                    "documents_chercher" => return format!("recherche dans les documents{}", if js::truthy(&av("recherche")) { format!(" : {}", d.sh(&av("recherche"), 30)) } else { String::new() }),
                    "approve" => return format!("demande d'autorisation : {}", a("tool_name")),
                    _ => {}
                }
            }
            if server == "system" {
                match mcp_tool.as_str() {
                    "ouvrir_application" => return format!("lancement : {}", a("nom")),
                    "fermer_application" => return format!("fermeture : {}", a("nom")),
                    "lister_fenetres" => return "liste des fenêtres".into(),
                    "activer_fenetre" => return format!("fenêtre : {}", d.sh(&av("titre"), 30)),
                    "ouvrir_url" => return format!("navigation : {}", d.sh(&av("url"), 30)),
                    "ouvrir_dossier" => return format!("explorateur : {}", d.base(&av("chemin"))),
                    "presse_papier_lire" => return "lecture presse-papier".into(),
                    "presse_papier_ecrire" => return format!("copie presse-papier : {}", d.sh(&av("texte"), 25)),
                    "capture_ecran" => return "capture d'écran".into(),
                    "dessiner" => return format!("dessin : {}", a("sujet")),
                    "projets" => return if js::truthy(&av("nom")) { format!("projet : {}", d.sh(&av("nom"), 30)) } else { "liste des projets".into() },
                    "maison" => return format!("maison : {} {}", a("action"), a("nom")),
                    "lumiere" => return format!("lumière : {}{}{}", a("action"), if !av("valeur").is_null() { format!(" {}", js::s(&av("valeur"))) } else { String::new() }, if js::truthy(&av("lampe")) { format!(" ({})", a("lampe")) } else { String::new() }),
                    "volume" => return format!("volume : {}", js::s(&or(&args, &["action", "niveau"]))),
                    "luminosite" => return format!("luminosité : {}%", js::s(&av("pourcentage"))),
                    "verrouiller" => return "verrouillage session".into(),
                    "veille" => return "mise en veille".into(),
                    _ => {}
                }
            }
            if server == "google" {
                match mcp_tool.as_str() {
                    "mail_derniers" => return "derniers emails".into(),
                    "mail_chercher" => return format!("recherche emails : {}", d.sh(&av("requete"), 25)),
                    "mail_lire" => return "lecture d’un email".into(),
                    "mail_envoyer" => return format!("envoi email : {}", d.sh(&av("destinataire"), 25)),
                    "agenda_jour" => return format!("agenda : {}", if js::truthy(&av("date")) { a("date") } else { "aujourd'hui".into() }),
                    "agenda_semaine" => return "agenda des 7 jours".into(),
                    "agenda_creer" => return format!("nouveau rendez-vous : {}", d.sh(&av("titre"), 25)),
                    "google_statut" => return "état compte Google".into(),
                    "google_configurer" => return "configuration Google".into(),
                    _ => {}
                }
            }
            format!("{}{}", if mcp_tool.is_empty() { "outil MCP".to_string() } else { mcp_tool }, if server.is_empty() { String::new() } else { format!(" ({server})") })
        }
        _ => {
            let iv = |k: &str| js::get(input, k).clone();
            if let Some(action) = tool.strip_prefix("mcp__system__") {
                match action {
                    "ouvrir_application" => return format!("lancement : {}", g("nom")),
                    "fermer_application" => return format!("fermeture : {}", g("nom")),
                    "lister_fenetres" => return "liste des fenêtres".into(),
                    "activer_fenetre" => return format!("fenêtre : {}", d.sh(&iv("titre"), 30)),
                    "ouvrir_url" => return format!("navigation : {}", d.sh(&iv("url"), 30)),
                    "ouvrir_dossier" => return format!("explorateur : {}", d.base(&iv("chemin"))),
                    "presse_papier_lire" => return "lecture presse-papier".into(),
                    "presse_papier_ecrire" => return format!("copie presse-papier : {}", d.sh(&iv("texte"), 25)),
                    "capture_ecran" => return "capture d'écran".into(),
                    "volume" => return format!("volume : {}", js::s(&or(input, &["action", "niveau"]))),
                    "luminosite" => return format!("luminosité : {}%", js::s(&iv("pourcentage"))),
                    "fenetre_active" => return "détection fenêtre active".into(),
                    "etat_bureau" => return format!("état du bureau{}", if js::truthy(&iv("ecran")) { format!(" (écran {})", g("ecran")) } else { String::new() }),
                    "fenetre" => return format!("fenêtre {} : {}{}", d.sh(&iv("cible"), 25), g("action"), if js::truthy(&iv("ecran")) { format!(" → écran {}", g("ecran")) } else { String::new() }),
                    "fenetre_fermer" => return format!("fermeture de la fenêtre {}", d.sh(&iv("cible"), 25)),
                    "onglets" => {
                        let act = if g("action").is_empty() { "lister".to_string() } else { g("action") };
                        return format!("Chrome : {act}{}{}", if js::truthy(&iv("page")) { format!(" {}", d.sh(&iv("page"), 25)) } else { String::new() }, if js::truthy(&iv("url")) { format!(" {}", d.sh(&iv("url"), 30)) } else { String::new() });
                    }
                    "ui_lister" => return format!("boutons de {}", d.sh(&Value::String(if g("application").is_empty() { "la fenêtre active".into() } else { g("application") }), 25)),
                    "ui_agir" => {
                        let app = d.sh(&Value::String(if g("application").is_empty() { "la fenêtre active".into() } else { g("application") }), 20);
                        let what = if g("action") == "remplir" {
                            format!("remplir « {} »", d.sh(&iv("champ"), 20))
                        } else {
                            let c = if js::truthy(&iv("cible")) { g("cible") } else { g("numero") };
                            format!("cliquer sur « {} »", d.sh(&Value::String(c), 25))
                        };
                        return format!("dans {app} : {what}");
                    }
                    "onglet_agir" => {
                        let what = if g("action") == "remplir" { format!("remplir « {} »", d.sh(&iv("champ"), 20)) } else { format!("cliquer sur « {} »", d.sh(&iv("texte"), 25)) };
                        return format!("Chrome, page {} : {what}", d.sh(&iv("page"), 15));
                    }
                    "sourdine_contextuelle" => return format!("sourdine : {}", if g("mode").is_empty() { "consultation".into() } else { g("mode") }),
                    "media" => return format!("musique : {}{}{}", if g("action").is_empty() { "état".into() } else { g("action") }, if js::truthy(&iv("requete")) { format!(" « {} »", g("requete")) } else { String::new() }, if js::truthy(&iv("app")) { format!(" ({})", g("app")) } else { String::new() }),
                    "verrouiller" => return "verrouillage session".into(),
                    "veille" => return "mise en veille".into(),
                    "chercher_fichiers" => return format!("recherche fichiers : {}", d.sh(&iv("requete"), 25)),
                    "ouvrir_fichier" => return format!("ouverture : {}", d.base(&iv("chemin"))),
                    "saisir_texte" => return format!("saisie : {}", d.sh(&iv("texte"), 25)),
                    "dessiner" => return format!("dessin : {}", g("sujet")),
                    "projets" => return if js::truthy(&iv("nom")) { format!("projet : {}", d.sh(&iv("nom"), 30)) } else { "liste des projets".into() },
                    "maison" => return format!("maison : {} {}", g("action"), g("nom")),
                    "lumiere" => return format!("lumière : {}{}{}", g("action"), if !iv("valeur").is_null() { format!(" {}", js::s(&iv("valeur"))) } else { String::new() }, if js::truthy(&iv("lampe")) { format!(" ({})", g("lampe")) } else { String::new() }),
                    "tmux" => return format!("tmux : {}", g("action")),
                    "tmux_claude" => return format!("tmux : {}", d.sh(&iv("commande"), 40)),
                    "terminal_dicter" => return format!("terminal {}: {}", if js::truthy(&iv("cible")) { format!("({}) ", d.sh(&iv("cible"), 25)) } else { String::new() }, d.sh(&iv("texte"), 40)),
                    "tmux_dicter" | "tmux_shell" => return format!("tmux {}: {}", if js::truthy(&iv("cible")) { format!("({}) ", d.sh(&iv("cible"), 25)) } else { String::new() }, d.sh(&iv("texte"), 40)),
                    "tmux_fermer" => return format!("fermeture tmux : {}", d.sh(&iv("cible"), 40)),
                    "programmation_action" => return format!("programmation : {}", g("action")),
                    _ => {}
                }
            }
            if let Some(action) = tool.strip_prefix("mcp__google__") {
                match action {
                    "mail_derniers" => return "derniers emails".into(),
                    "mail_chercher" => return format!("recherche emails : {}", d.sh(&iv("requete"), 25)),
                    "mail_lire" => return "lecture d’un email".into(),
                    "mail_envoyer" => return format!("envoi email : {}", d.sh(&iv("destinataire"), 25)),
                    "agenda_jour" => return format!("agenda : {}", if js::truthy(&iv("date")) { g("date") } else { "aujourd'hui".into() }),
                    "agenda_semaine" => return "agenda des 7 jours".into(),
                    "agenda_creer" => return format!("nouveau rendez-vous : {}", d.sh(&iv("titre"), 25)),
                    "google_statut" => return "état compte Google".into(),
                    "google_configurer" => return "configuration Google".into(),
                    _ => {}
                }
            }
            let p = d.base(&or(input, PATH_KEYS));
            if !p.is_empty() {
                return format!("{label} : {p}");
            }
            let cmd = d.sh(&or(input, &["CommandLine", "command", "query", "Query", "url", "Url"]), 50);
            if !cmd.is_empty() {
                return format!("{label} : {cmd}");
            }
            if let Some(obj) = input.as_object() {
                let pairs: Vec<String> = obj
                    .iter()
                    .filter(|(k, v)| !matches!(k.as_str(), "Content" | "content" | "Replacement" | "Target" | "new_string" | "old_string") && js::truthy(v))
                    .take(3)
                    .map(|(k, v)| format!("{k}={}", d.sh(v, 40)))
                    .collect();
                if !pairs.is_empty() {
                    return format!("{label} ({})", pairs.join(", "));
                }
            }
            label
        }
    }
}

fn inside(workspace: &str, target: &str) -> bool {
    if target.is_empty() {
        return true;
    }
    let root = super::zones::resolve(workspace);
    let resolved = std::path::Path::new(&root).join(target);
    let resolved = super::zones::resolve(&resolved.to_string_lossy());
    resolved == root || resolved.starts_with(&format!("{root}\\"))
}

/// Une cible désigne-t-elle un fichier interne de Jarvis (mémoire, journal, réglages) ?
pub fn forbidden(input: &J, guarded: &[String]) -> bool {
    let mut targets: Vec<String> = PATH_KEYS
        .iter()
        .chain(["command", "pattern", "CommandLine"].iter())
        .map(|k| js::get(input, k).clone())
        .collect::<Vec<_>>()
        .into_iter()
        .chain([
            input.get("Arguments").map(|a| js::get(a, "reference").clone()).unwrap_or(Value::Null),
            input.get("Arguments").map(|a| js::get(a, "texte").clone()).unwrap_or(Value::Null),
        ])
        .filter(js::truthy)
        .map(|v| js::s(&v).to_lowercase().replace('/', "\\"))
        .collect();
    targets.dedup();
    guarded.iter().any(|dir| {
        let needle = dir.to_lowercase().replace('/', "\\");
        targets.iter().any(|target| {
            if target.contains(&needle) {
                return true;
            }
            // Référence au dossier racine des données (« data », « history »).
            let base_needle = needle.rsplit('\\').next().unwrap_or("");
            if base_needle == "data" || base_needle == "history" {
                let re = regex::Regex::new(&format!(r#"(?i)(^|[\\/\s"'`]){}([\\/\s"'`]|$)"#, base_needle)).unwrap();
                if re.is_match(target) {
                    return true;
                }
            }
            false
        })
    })
}

const SYSTEM_PLAIN: &[&str] = &[
    "ouvrir_application", "media", "volume", "luminosite", "lister_fenetres", "activer_fenetre", "ouvrir_url", "ouvrir_dossier",
    "presse_papier_lire", "presse_papier_ecrire", "capture_ecran", "lire_ecran_ocr", "fenetre_active", "sourdine_contextuelle",
    "chercher_fichiers", "ouvrir_fichier", "saisir_texte", "dessiner", "projets", "lumiere", "maison", "fermer_application", "verrouiller", "veille",
];
const GOOGLE_PLAIN: &[&str] = &["mail_derniers", "mail_chercher", "mail_lire", "mail_envoyer", "agenda_jour", "agenda_semaine", "agenda_creer", "google_statut", "google_configurer"];
const JARVIS_PLAIN: &[&str] = &["approve", "memoire_lire", "memoire_ajouter", "historique_lire", "memoire_oublier", "demander_precision", "memoire_effacer", "reunions_chercher", "reunion_lire", "listes_chercher", "listes_lire", "documents_chercher", "savoir_chercher"];

pub fn target_of(input: &J) -> String {
    let direct = js::s(&or(input, PATH_KEYS));
    if !direct.is_empty() {
        return direct;
    }
    if let Some(obj) = input.as_object() {
        for (k, v) in obj {
            if matches!(k.as_str(), "Content" | "content" | "Replacement" | "Target" | "new_string" | "old_string" | "command" | "CommandLine") {
                continue;
            }
            if let Some(s) = v.as_str() {
                let s = s.trim();
                if !s.is_empty() && s.len() < 400 && !s.contains('\n') && (s.contains('\\') || s.contains('/') || s.ends_with(".json") || s.ends_with(".md") || s.ends_with(".txt")) {
                    return s.to_string();
                }
            }
        }
    }
    String::new()
}

/// Verdict de la politique historique : « allow », « voice », « code » ou « deny ».
pub fn evaluate(tool: &str, input: &J, workspace: Option<&str>, zones: Option<&Zones>) -> String {
    let mut tool = tool.to_string();
    if let Some(r) = tool.strip_prefix("system:") {
        tool = format!("mcp__system__{r}");
    } else if let Some(r) = tool.strip_prefix("google:") {
        tool = format!("mcp__google__{r}");
    } else if let Some(r) = tool.strip_prefix("jarvis:") {
        tool = format!("mcp__jarvis__{r}");
    } else if SYSTEM_PLAIN.contains(&tool.as_str()) {
        tool = format!("mcp__system__{tool}");
    } else if GOOGLE_PLAIN.contains(&tool.as_str()) {
        tool = format!("mcp__google__{tool}");
    } else if JARVIS_PLAIN.contains(&tool.as_str()) {
        tool = format!("mcp__jarvis__{tool}");
    }
    let t = tool.as_str();

    if t == "call_mcp_tool" {
        let server = js::gs(input, "ServerName");
        let mcp_tool = js::gs(input, "ToolName");
        if server == "jarvis" {
            if ["memoire_lire", "memoire_ajouter", "historique_lire", "reunions_chercher", "reunion_lire", "listes_chercher", "listes_lire", "documents_chercher", "savoir_chercher", "approve"].contains(&mcp_tool.as_str()) {
                return "allow".into();
            }
            if mcp_tool == "memoire_oublier" || mcp_tool == "memoire_effacer" {
                return "code".into();
            }
        }
        if server == "system" {
            return if ["fermer_application", "verrouiller", "veille"].contains(&mcp_tool.as_str()) { "code" } else { "allow" }.into();
        }
        if server == "google" {
            return if ["mail_envoyer", "agenda_creer", "google_configurer"].contains(&mcp_tool.as_str()) { "code" } else { "allow" }.into();
        }
        return "allow".into();
    }
    // Oubli et purge de la mémoire : jamais sans code.
    if t == "mcp__jarvis__memoire_oublier" || t == "mcp__jarvis__memoire_effacer" {
        return "code".into();
    }
    if t.starts_with("mcp__jarvis__") {
        return "allow".into();
    }
    if ["mcp__system__fermer_application", "mcp__system__verrouiller", "mcp__system__veille"].contains(&t) {
        return "code".into();
    }
    // Fichier trouvé par la recherche locale : une zone interdite reste interdite.
    if (t == "mcp__system__ouvrir_fichier" || t == "mcp__system__chercher_fichiers") && zones.is_some() && js::truthy(js::get(input, "chemin")) {
        if zones.unwrap().check(&js::gs(input, "chemin"), "read") == "forbidden" {
            return "deny".into();
        }
    }
    if t.starts_with("mcp__system__") {
        return "allow".into();
    }
    if ["mcp__google__mail_envoyer", "mcp__google__agenda_creer", "mcp__google__google_configurer"].contains(&t) {
        return "code".into();
    }
    if t.starts_with("mcp__google__") {
        return "allow".into();
    }
    if SHELL_TOOLS.contains(&t) {
        return "code".into();
    }
    // Chemin relatif : l'agent tourne dans le dossier de travail, pas dans celui du noyau.
    let target = match (target_of(input), workspace) {
        (p, Some(ws)) if !p.is_empty() && std::path::Path::new(&p).is_relative() => std::path::Path::new(ws).join(&p).to_string_lossy().into_owned(),
        (p, _) => p,
    };
    if WRITE_TOOLS.contains(&t) {
        if let (Some(z), false) = (zones, target.is_empty()) {
            let perm = z.check(&target, "write");
            return match perm.as_str() {
                "forbidden" => "deny",
                "free" => "allow",
                _ => "voice",
            }
            .into();
        }
        if let (Some(ws), false) = (workspace, target.is_empty()) {
            if inside(ws, &target) {
                return "allow".into();
            }
        }
        return "code".into();
    }
    if READ_TOOLS.contains(&t) {
        if let (Some(z), false) = (zones, target.is_empty()) {
            let perm = z.check(&target, "read");
            return match perm.as_str() {
                "forbidden" => "deny",
                "free" => "allow",
                _ => "code",
            }
            .into();
        }
        if let (Some(ws), false) = (workspace, target.is_empty()) {
            return if inside(ws, &target) { "allow" } else { "code" }.into();
        }
        return "allow".into();
    }
    if AUTO_ALLOWED.contains(&t) {
        return "allow".into();
    }
    "code".into()
}

pub fn security_level(verdict: &str) -> (&'static str, &'static str) {
    match verdict {
        "allow" => ("anodin", "Anodin (automatique)"),
        "voice" => ("modere", "Modéré (accord oral « oui »)"),
        "code" => ("critique", "Critique (code écrit requis)"),
        _ => ("interdit", "Interdit"),
    }
}

/// Formulation lue à voix haute au moment de demander l'accord.
pub fn spoken_request(tool: &str, input: &J, zones: Option<&Zones>) -> String {
    let d = D { full: false };
    let target = target_of(input);
    let zone = match (zones, target.is_empty()) {
        (Some(z), false) => z.find_zone(&target),
        _ => None,
    };
    let zone_tag = zone.map(|z| format!(" dans la zone {}", js::gs(&z, "name"))).unwrap_or_default();
    let g = |k: &str| js::gs(input, k);
    match tool {
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "write_to_file" | "write_file" | "create_file" | "edit_file" | "replace_file_content" | "multi_replace_file_content" => {
            let b = d.base(&Value::String(target.clone()));
            if b.is_empty() { format!("Écriture de fichier{zone_tag}") } else { format!("Écriture dans le fichier {b}{zone_tag}") }
        }
        "Read" | "view_file" | "read_file" | "view_file_outline" | "view_code_item" => {
            let b = d.base(&Value::String(target.clone()));
            if b.is_empty() { format!("Lecture de fichier{zone_tag}") } else { format!("Lecture du fichier {b}{zone_tag}") }
        }
        "Bash" | "run_command" | "send_command_input" => format!("Commande système : {}", d.sh(&or(input, &["CommandLine", "command", "Input"]), 50)),
        "mcp__jarvis__memoire_oublier" => "Effacement d'une information mémorisée".into(),
        "mcp__jarvis__memoire_effacer" => "Effacement complet de la mémoire".into(),
        "mcp__system__fermer_application" => format!("Fermeture de l'application {}", g("nom")),
        "mcp__system__verrouiller" => "Verrouillage de la session Windows".into(),
        "mcp__system__veille" => "Mise en veille de l'ordinateur".into(),
        "mcp__system__alimentation" => if g("action") == "redemarrer" { "Redémarrage de l'ordinateur" } else { "Extinction de l'ordinateur" }.into(),
        "mcp__system__vider_corbeille" => "Vidage de la corbeille".into(),
        "mcp__system__tmux_shell" => format!("Commande dans tmux ({}) : {}", if g("cible").is_empty() { "terminal".into() } else { g("cible") }, d.sh(js::get(input, "texte"), 60)),
        "mcp__system__tmux_fermer" => format!("Fermeture tmux : {}", g("cible")),
        "mcp__system__tmux_claude" => format!("Lancement de Claude Code dans tmux : {}", d.sh(js::get(input, "commande"), 60)),
        "mcp__google__mail_envoyer" => format!("Envoi d'un email à {}", g("destinataire")),
        "mcp__google__agenda_creer" => format!("Création du rendez-vous {} le {}", g("titre"), g("date")),
        "mcp__google__google_configurer" => "Configuration des identifiants Google".into(),
        "call_mcp_tool" => {
            let server = g("ServerName");
            let tn = g("ToolName");
            let args = js::get(input, "Arguments").clone();
            if server == "jarvis" && tn == "memoire_oublier" {
                return "Effacement d'une information mémorisée".into();
            }
            if server == "jarvis" && tn == "memoire_effacer" {
                return "Effacement complet de la mémoire".into();
            }
            if server == "system" {
                match tn.as_str() {
                    "fermer_application" => return format!("Fermeture de l'application {}", js::gs(&args, "nom")),
                    "verrouiller" => return "Verrouillage de la session Windows".into(),
                    "veille" => return "Mise en veille de l'ordinateur".into(),
                    _ => {}
                }
            }
            if server == "google" {
                match tn.as_str() {
                    "mail_envoyer" => return format!("Envoi d'un email à {}", js::gs(&args, "destinataire")),
                    "agenda_creer" => return format!("Création du rendez-vous {} le {}", js::gs(&args, "titre"), js::gs(&args, "date")),
                    "google_configurer" => return "Configuration des identifiants Google".into(),
                    _ => {}
                }
            }
            format!("Action de l'outil MCP {}", if tn.is_empty() { tool.to_string() } else { tn })
        }
        _ => {
            if let Some(action) = tool.strip_prefix("system:") {
                match action {
                    "fermer_application" => return format!("Fermeture de l'application {}", g("nom")),
                    "verrouiller" => return "Verrouillage de la session Windows".into(),
                    "veille" => return "Mise en veille de l'ordinateur".into(),
                    _ => {}
                }
            }
            if let Some(action) = tool.strip_prefix("google:") {
                match action {
                    "mail_envoyer" => return format!("Envoi d'un email à {}", g("destinataire")),
                    "agenda_creer" => return format!("Création du rendez-vous {} le {}", g("titre"), g("date")),
                    "google_configurer" => return "Configuration des identifiants Google".into(),
                    _ => {}
                }
            }
            format!("Action {}{zone_tag}", label(tool).unwrap_or(tool))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn zones(name: &str) -> (Zones, String) {
        let d = std::env::temp_dir().join(format!("jarvis-test-policy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ws = d.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        (Zones::new(d.join("zones.json"), ws.clone()), ws.to_string_lossy().into_owned())
    }

    #[test]
    fn chemin_relatif_ecrit_dans_le_dossier_de_travail() {
        let (z, ws) = zones("rel");
        assert_eq!(evaluate("Write", &json!({ "file_path": "notes/a.md" }), Some(&ws), Some(&z)), "allow");
    }

    #[test]
    fn chemin_relatif_qui_sort_du_dossier_de_travail_n_est_pas_libre() {
        let (z, ws) = zones("sortie");
        assert_ne!(evaluate("Write", &json!({ "file_path": "..\\..\\hors.md" }), Some(&ws), Some(&z)), "allow");
    }

    #[test]
    fn ecriture_hors_de_toute_zone_est_refusee() {
        let (z, ws) = zones("hors");
        assert_eq!(evaluate("Write", &json!({ "file_path": "Z:\\inconnu\\a.md" }), Some(&ws), Some(&z)), "deny");
    }

    #[test]
    fn la_lecture_reste_libre_dans_le_dossier_de_travail() {
        let (z, ws) = zones("lecture");
        assert_eq!(evaluate("Read", &json!({ "file_path": "a.md" }), Some(&ws), Some(&z)), "allow");
    }

    #[test]
    fn outils_agy_v2_reconnus_et_decrits() {
        let (z, ws) = zones("agy2");
        assert_eq!(evaluate("write_file", &json!({ "Path": "savoir/sujet/dossier.json" }), Some(&ws), Some(&z)), "allow");
        assert_eq!(evaluate("edit_file", &json!({ "Path": "savoir/sujet/dossier.json" }), Some(&ws), Some(&z)), "allow");
        assert_eq!(evaluate("read_file", &json!({ "Path": "savoir/sujet/dossier.json" }), Some(&ws), Some(&z)), "allow");
        assert_eq!(
            describe_full("write_file", &json!({ "Path": "E:\\workspace\\savoir\\dossier.json" })),
            "écriture : E:\\workspace\\savoir\\dossier.json"
        );
    }
}
