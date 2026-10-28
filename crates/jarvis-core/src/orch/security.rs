// Sécurité paramétrable (orchestrator/security.js).
//
// Catalogue des types d'actions soumis à un accord, avec leur niveau :
//   allow : accord direct · voice : accord oral (« oui ») · code : code à 4 chiffres · deny : interdit
// Le niveau par défaut reproduit la politique historique (policy.rs). Les réglages vivent dans
// data/security.json, hors de portée des modèles. Un refus de zone reste un refus ; abaisser un
// niveau exige un code (géré par l'administration), resserrer est libre.

use std::cell::RefCell;
use std::path::PathBuf;

use indexmap_lite::IndexMap;
use serde_json::{json, Value};

use crate::js::{self, J};

mod indexmap_lite {
    pub type IndexMap<K, V> = serde_json::Map<K, V>;
}

pub const LEVELS: &[&str] = &["allow", "voice", "code", "deny"];

pub fn rank(level: &str) -> i32 {
    match level {
        "allow" => 0,
        "voice" => 1,
        "code" => 2,
        "deny" => 3,
        _ => 2,
    }
}

pub fn level_label(level: &str) -> &'static str {
    match level {
        "allow" => "Accord direct",
        "voice" => "Accord oral (« oui »)",
        "code" => "Code à 4 chiffres",
        _ => "Interdit",
    }
}

type E = (&'static str, &'static str, &'static str, &'static str, &'static str, &'static str);

// (clé, groupe, libellé, niveau, risque, description)
pub const CATALOG: &[E] = &[
    ("shell", "Fichiers & système", "Commandes système (terminal)", "code", "élevé", "Bash (Claude Code), run_command (AGY) : exécute n’importe quelle commande Windows."),
    ("ecriture_zone", "Fichiers & système", "Écriture dans une zone déclarée", "voice", "moyen", "Créer ou modifier un fichier dans une zone dont l’écriture est « code exigé ». Le dossier de travail et les zones en écriture libre ne demandent rien ; hors zone, l’écriture reste interdite."),
    ("lecture_hors_zone", "Fichiers & système", "Lecture hors des zones déclarées", "code", "moyen", "Lire un fichier ailleurs sur le disque."),
    ("sous_agent", "Fichiers & système", "Lancement de sous-agents", "code", "moyen", "Task (Claude Code), invoke_subagent (AGY) : délégation à un autre agent."),
    ("web", "Fichiers & système", "Recherche et consultation web", "allow", "faible", "WebSearch, WebFetch, search_web, read_url_content."),
    ("autre", "Fichiers & système", "Outils non répertoriés", "code", "inconnu", "Tout outil que Jarvis ne connaît pas encore."),
    ("memoire_oublier", "Mémoire", "Oubli d’un souvenir (demandé à Jarvis)", "code", "moyen", "memoire_oublier : Jarvis supprime une information de sa mémoire."),
    ("memoire_purge", "Mémoire", "Effacement complet de la mémoire", "code", "élevé", "Voix (« efface ta mémoire »), bouton du HUD, ou outil memoire_effacer."),
    ("mcp__system__fermer_application", "Windows", "Fermer une application", "code", "moyen", ""),
    ("mcp__system__verrouiller", "Windows", "Verrouiller la session", "code", "moyen", ""),
    ("mcp__system__veille", "Windows", "Mettre l’ordinateur en veille", "code", "moyen", ""),
    ("mcp__system__alimentation", "Windows", "Éteindre ou redémarrer l’ordinateur", "code", "élevé", "Extinction ou redémarrage dans 30 secondes (« annule l’extinction » l’arrête)."),
    ("mcp__system__alimentation_annuler", "Windows", "Annuler une extinction programmée", "allow", "faible", ""),
    ("mcp__system__vider_corbeille", "Windows", "Vider la corbeille", "voice", "moyen", ""),
    ("mcp__system__ouvrir_parametres", "Windows", "Ouvrir une page des Paramètres Windows", "allow", "faible", ""),
    ("mcp__jarvis__listes_chercher", "Mémoire", "Chercher dans mes listes à fiches", "allow", "moyen", "Lecture seule : vinyles, Lego… avec prix, lieu et date d'achat. L'IA ne modifie jamais une liste."),
    ("mcp__jarvis__documents_chercher", "Mémoire", "Chercher dans mes documents", "allow", "moyen", "Lecture seule : uniquement les dossiers que tu as choisis (Administration › Documents). Les extraits lus sont transmis à l'IA pour répondre."),
    ("mcp__jarvis__savoir_chercher", "Mémoire", "Chercher dans mes dossiers Data", "allow", "faible", "Lecture seule : les dossiers de recherche que tu as demandés (« fais un dossier sur… »). L'IA ne les modifie jamais."),
    ("mcp__jarvis__listes_lire", "Mémoire", "Lire mes listes à fiches", "allow", "moyen", "Lecture seule : la liste de mes listes, leurs fiches et le total dépensé."),
    ("mcp__system__ouvrir_application", "Windows", "Ouvrir une application", "allow", "faible", ""),
    ("mcp__system__ouvrir_url", "Windows", "Ouvrir une page web", "allow", "faible", ""),
    ("mcp__system__ouvrir_dossier", "Windows", "Ouvrir un dossier", "allow", "faible", ""),
    ("mcp__system__etat_bureau", "Windows", "Voir les fenêtres, écrans et applis ouvertes", "allow", "faible", ""),
    ("mcp__system__fenetre", "Windows", "Afficher, réduire, agrandir ou déplacer une fenêtre", "allow", "faible", ""),
    ("mcp__system__fenetre_fermer", "Windows", "Fermer une fenêtre", "voice", "moyen", ""),
    ("mcp__system__onglets", "Windows", "Onglets de Chrome : voir, aller sur, ouvrir, recharger, fermer, lire", "allow", "faible", ""),
    ("mcp__system__ui_lister", "Windows", "Voir les boutons d’une application", "allow", "faible", ""),
    ("mcp__system__ui_agir", "Windows", "Cliquer ou écrire dans une application (sauf applis de confiance ; jamais dans les applis sensibles)", "voice", "moyen", ""),
    ("mcp__system__onglet_agir", "Windows", "Cliquer ou remplir un champ dans une page de Chrome", "voice", "moyen", ""),
    ("mcp__system__capture_ecran", "Windows", "Capture d’écran", "allow", "moyen", "Uniquement sur demande explicite (consigne de Jarvis)."),
    ("mcp__system__lire_ecran_ocr", "Windows", "Lecture de l’écran (OCR)", "allow", "moyen", ""),
    ("mcp__system__dessiner", "Windows", "Dessiner dans Paint", "allow", "faible", "Crée un nouveau dessin (mouton, chat, maison…) dans workspace/dessins et l’ouvre dans Paint. Ni souris ni clavier pilotés, rien n’est écrasé."),
    ("mcp__system__maison", "Maison", "Piloter lampes Hue et prises connectées", "allow", "faible", "Allumer, éteindre ou régler les lampes Philips Hue (pont local) et les prises Shelly ou Kasa déclarées dans Administration › Maison. Réseau local seulement."),
    ("mcp__system__lumiere", "Maison", "Piloter les lumières (Yeelight)", "allow", "faible", "Allumer, éteindre, régler la luminosité, la couleur ou une ambiance des ampoules Yeelight du réseau local."),
    ("mcp__system__projets", "Windows", "Consulter mes projets", "allow", "faible", "Liste et détail des projets du dossier réglé dans Administration › Projets (description, technologies, état git). Lecture seule."),
    ("mcp__system__presse_papier_lire", "Windows", "Lire le presse-papier", "allow", "moyen", ""),
    ("mcp__system__presse_papier_ecrire", "Windows", "Écrire dans le presse-papier", "allow", "faible", ""),
    ("mcp__system__volume", "Windows", "Régler le volume", "allow", "faible", ""),
    ("mcp__system__luminosite", "Windows", "Régler la luminosité", "allow", "faible", ""),
    ("mcp__system__media", "Windows", "Contrôle de la musique (lecture, pause, suivant)", "allow", "faible", ""),
    ("mcp__system__chercher_fichiers", "Windows", "Chercher un fichier ou un dossier", "allow", "faible", "Recherche par nom dans les zones déclarées (hors zone Projets), sans lire le contenu."),
    ("mcp__system__ouvrir_fichier", "Windows", "Ouvrir un fichier trouvé", "allow", "faible", "Ouvre un document avec son application. Un exécutable ou un script n’est jamais lancé : son dossier est affiché."),
    ("mcp__system__saisir_texte", "Windows", "Taper du texte dicté dans un document", "allow", "moyen", "Uniquement dans une fenêtre de traitement de texte (Google Docs, Word, Bloc-notes…)."),
    ("mcp__system__tmux", "tmux", "tmux : ouvrir, naviguer, lire l’écran", "allow", "faible", "Session, fenêtres, volets, capture de ce qu’affiche le terminal, interruption (Échap / Ctrl+C)."),
    ("mcp__system__tmux_claude", "tmux", "tmux : lancer Claude Code", "allow", "élevé", "Lance la commande réglée (par défaut claude : Claude Code demande avant d'agir ; avec --dangerously-skip-permissions il agit sans demander, et chaque lancement est alors inscrit au journal)."),
    ("mcp__system__tmux_dicter", "tmux", "tmux : écrire à un agent (Claude Code…)", "allow", "moyen", "La phrase dite est tapée dans un volet où tourne un agent : c’est une consigne, pas une commande."),
    ("mcp__system__tmux_shell", "tmux", "tmux : commande dans un terminal", "code", "élevé", "Lancer une commande, ou taper dans un volet où tourne un shell ou un autre programme : c’est une commande système."),
    ("mcp__system__terminal_dicter", "tmux", "Voix → terminal au premier plan", "allow", "moyen", "Terminal devant (tmux, cmd, PowerShell) ou « 🎙 Dicter ici » : ce que tu dis y est écrit, à la suite, sans jamais appuyer sur Entrée."),
    ("mcp__system__terminal_valider", "tmux", "Voix → terminal : « je valide le prompt »", "allow", "élevé", "Appuie sur Entrée dans le terminal dicté : le message part à Claude Code, la commande d’un shell s’exécute."),
    ("mcp__system__programmation_action", "Programmations", "Programmation : éteindre, redémarrer, veille, verrouiller", "allow", "élevé", "Exécutée seulement après 120 secondes de compte à rebours affiché dans le cœur, annulable (« annule », Échap, STOP). « Interdit » : aucune programmation sensible ne s’exécute."),
    ("mcp__system__tmux_fermer", "tmux", "tmux : fermer une fenêtre ou la session", "voice", "moyen", "Arrête ce qui y tourne (une session Claude Code en cours est perdue)."),
    ("mcp__google__mail_envoyer", "Google", "Envoyer un email", "code", "élevé", ""),
    ("mcp__google__agenda_creer", "Google", "Créer un rendez-vous", "code", "moyen", ""),
    ("mcp__google__google_configurer", "Google", "Configurer le compte Google", "code", "élevé", ""),
    ("mcp__google__mail_derniers", "Google", "Lire les derniers emails", "allow", "moyen", ""),
    ("mcp__google__mail_chercher", "Google", "Chercher dans les emails", "allow", "moyen", ""),
    ("mcp__google__mail_lire", "Google", "Lire le contenu d’un email", "allow", "moyen", ""),
    ("mcp__google__agenda_jour", "Google", "Lire l’agenda du jour", "allow", "faible", ""),
    ("mcp__google__agenda_semaine", "Google", "Lire l’agenda de la semaine", "allow", "faible", ""),
    ("mcp__google__google_statut", "Google", "Vérifier l’état de la connexion Google", "allow", "faible", ""),
];

const SHELL_TOOLS: &[&str] = &["Bash", "run_command", "send_command_input"];
const SUBAGENT_TOOLS: &[&str] = &["Task", "invoke_subagent", "define_subagent", "manage_subagents", "send_message"];
const WEB_TOOLS: &[&str] = &["WebSearch", "WebFetch", "search_web", "read_url_content"];
const WRITE_TOOLS: &[&str] = &[
    "Write", "Edit", "MultiEdit", "NotebookEdit",
    "write_to_file", "write_file", "create_file", "edit_file", "replace_file_content", "multi_replace_file_content",
];
const READ_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "NotebookRead", "LS",
    "view_file", "read_file", "view_file_outline", "view_code_item", "find_by_name", "grep_search", "list_dir",
];

pub const SYSTEM_TOOLS: &[&str] = &[
    "ouvrir_application", "fermer_application", "lister_fenetres", "activer_fenetre", "ouvrir_url", "ouvrir_dossier",
    "presse_papier_lire", "presse_papier_ecrire", "capture_ecran", "lire_ecran_ocr", "volume", "luminosite", "verrouiller",
    "veille", "fenetre_active", "etat_bureau", "fenetre", "fenetre_fermer", "onglets", "onglet_agir", "ui_lister", "ui_agir",
    "sourdine_contextuelle", "media", "chercher_fichiers", "ouvrir_fichier", "saisir_texte", "dessiner", "projets", "lumiere", "maison",
];
pub const GOOGLE_TOOLS: &[&str] = &["mail_derniers", "mail_chercher", "mail_lire", "mail_envoyer", "agenda_jour", "agenda_semaine", "agenda_creer", "google_statut", "google_configurer"];
pub const JARVIS_TOOLS: &[&str] = &["approve", "memoire_lire", "memoire_ajouter", "historique_lire", "memoire_oublier", "demander_precision", "memoire_effacer", "reunions_chercher", "reunion_lire", "listes_chercher", "listes_lire", "documents_chercher", "savoir_chercher"];

/// Nom d'outil normalisé : l'appel indirect d'AGY (call_mcp_tool) ou sans préfixe devient mcp__serveur__outil.
pub fn normalize_tool(tool: &str, input: &J) -> (String, J) {
    if tool == "call_mcp_tool" && js::truthy(js::get(input, "ServerName")) && js::truthy(js::get(input, "ToolName")) {
        let args = input.get("Arguments").cloned().filter(|a| !a.is_null()).unwrap_or_else(|| json!({}));
        return (format!("mcp__{}__{}", js::gs(input, "ServerName"), js::gs(input, "ToolName")), args);
    }
    // Format « eager » d'AGY (mcp_serveur_outil) → mcp__serveur__outil
    if !tool.starts_with("mcp__") {
        if let Some(c) = crate::re!(r"^mcp_([a-zA-Z0-9_-]+?)_(.+)$").captures(tool) {
            return (format!("mcp__{}__{}", &c[1], &c[2]), input.clone());
        }
    }
    if let Some(r) = tool.strip_prefix("system:") {
        return (format!("mcp__system__{r}"), input.clone());
    }
    if let Some(r) = tool.strip_prefix("google:") {
        return (format!("mcp__google__{r}"), input.clone());
    }
    if let Some(r) = tool.strip_prefix("jarvis:") {
        return (format!("mcp__jarvis__{r}"), input.clone());
    }
    if SYSTEM_TOOLS.contains(&tool) {
        return (format!("mcp__system__{tool}"), input.clone());
    }
    if GOOGLE_TOOLS.contains(&tool) {
        return (format!("mcp__google__{tool}"), input.clone());
    }
    if JARVIS_TOOLS.contains(&tool) {
        return (format!("mcp__jarvis__{tool}"), input.clone());
    }
    (tool.to_string(), input.clone())
}

pub struct Security {
    file: PathBuf,
    overrides: RefCell<IndexMap<String, Value>>,
    // types d'actions ajoutés dynamiquement (outils des connecteurs MCP)
    extra: RefCell<Vec<J>>,
}

impl Security {
    pub fn new(file: PathBuf) -> Self {
        let s = Self { file, overrides: RefCell::new(IndexMap::new()), extra: RefCell::new(vec![]) };
        s.load();
        s
    }

    pub fn load(&self) {
        let mut out = IndexMap::new();
        if let Some(raw) = js::read_json(&self.file) {
            if let Some(Value::Object(o)) = raw.get("overrides") {
                for (k, v) in o {
                    if v.as_str().map(|l| LEVELS.contains(&l)).unwrap_or(false) {
                        out.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        *self.overrides.borrow_mut() = out;
    }

    fn save(&self) {
        let _ = js::write_json(&self.file, &json!({ "overrides": Value::Object(self.overrides.borrow().clone()) }));
    }

    /// Types d'actions supplémentaires (connecteurs MCP) : [{ key, group, label, level, description }].
    pub fn set_extra(&self, entries: Vec<J>) {
        *self.extra.borrow_mut() = entries;
    }

    pub fn entry(&self, key: &str) -> Option<J> {
        if let Some(e) = CATALOG.iter().find(|e| e.0 == key) {
            return Some(json!({ "key": e.0, "group": e.1, "label": e.2, "level": e.3, "risk": e.4, "description": e.5 }));
        }
        self.extra.borrow().iter().rev().find(|e| js::gs(e, "key") == key).cloned()
    }

    pub fn default_level(&self, key: &str) -> String {
        self.entry(key).map(|e| js::gs(&e, "level")).filter(|l| !l.is_empty()).unwrap_or_else(|| "code".into())
    }

    pub fn level_for(&self, key: &str) -> String {
        self.overrides.borrow().get(key).and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| self.default_level(key))
    }

    /// Clé de catalogue d'un appel d'outil, selon le verdict de base de la politique historique.
    pub fn key_for(&self, raw_tool: &str, raw_input: &J, base: &str) -> Option<String> {
        let (tool, _) = normalize_tool(raw_tool, raw_input);
        let t = tool.as_str();
        if SHELL_TOOLS.contains(&t) {
            return Some("shell".into());
        }
        if SUBAGENT_TOOLS.contains(&t) {
            return Some("sous_agent".into());
        }
        if WEB_TOOLS.contains(&t) {
            return Some("web".into());
        }
        if WRITE_TOOLS.contains(&t) {
            return (base == "voice").then(|| "ecriture_zone".into());
        }
        if READ_TOOLS.contains(&t) {
            return (base == "code").then(|| "lecture_hors_zone".into());
        }
        if t == "mcp__jarvis__memoire_oublier" {
            return Some("memoire_oublier".into());
        }
        if t == "mcp__jarvis__memoire_effacer" {
            return Some("memoire_purge".into());
        }
        if t.starts_with("mcp__jarvis__") {
            return None;
        }
        if t.starts_with("mcp__") && self.entry(t).is_some() {
            return Some(tool.clone());
        }
        if t.starts_with("mcp__") && !t.starts_with("mcp__system__") && !t.starts_with("mcp__google__") {
            return Some(tool.clone()); // outil d'un connecteur externe : niveau par défaut « code »
        }
        (base == "code").then(|| "autre".into())
    }

    /// Verdict final : la politique historique, puis le réglage de l'utilisateur.
    pub fn apply(&self, tool: &str, input: &J, base: &str) -> String {
        if base == "deny" {
            return "deny".into();
        }
        match self.key_for(tool, input, base) {
            None => base.to_string(),
            Some(key) => self.level_for(&key),
        }
    }

    pub fn is_lowering(&self, key: &str, level: &str) -> bool {
        rank(level) < rank(&self.level_for(key))
    }

    pub fn set(&self, key: &str, level: &str) -> Result<J, String> {
        if !LEVELS.contains(&level) {
            return Err(format!("niveau inconnu : {level}"));
        }
        if self.entry(key).is_none() {
            return Err(format!("type d'action inconnu : {key}"));
        }
        let before = self.level_for(key);
        if level == self.default_level(key) {
            self.overrides.borrow_mut().remove(key);
        } else {
            self.overrides.borrow_mut().insert(key.to_string(), json!(level));
        }
        self.save();
        Ok(json!({ "key": key, "before": before, "after": self.level_for(key) }))
    }

    pub fn list(&self) -> Vec<J> {
        let mut all: Vec<J> = CATALOG
            .iter()
            .map(|e| json!({ "key": e.0, "group": e.1, "label": e.2, "level": e.3, "risk": e.4, "description": e.5 }))
            .collect();
        // Map JS : une clé ajoutée deux fois ne figure qu'une fois, avec la dernière valeur.
        let mut seen = std::collections::HashSet::new();
        let extra: Vec<J> = self.extra.borrow().iter().rev().filter(|e| seen.insert(js::gs(e, "key"))).cloned().collect();
        all.extend(extra.into_iter().rev());
        let ov = self.overrides.borrow();
        all.iter()
            .map(|e| {
                let key = js::gs(e, "key");
                json!({
                    "key": key, "group": js::gs(e, "group"), "label": js::gs(e, "label"),
                    "description": js::gs(e, "description"), "risk": js::gs(e, "risk"),
                    "default": js::gs(e, "level"), "level": self.level_for(&key), "custom": ov.contains_key(&key)
                })
            })
            .collect()
    }
}
