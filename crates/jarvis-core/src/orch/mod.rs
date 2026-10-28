// Orchestrateur (orchestrator/index.js de la v3) : reçoit une demande vocale ou écrite, la traite sans IA
// quand c'est possible (routeur local), sinon la fait exécuter par Claude Code ou AGY ; diffuse
// l'avancement vers l'interface et bloque les actions sensibles tant que le code n'a pas été saisi.
// Un module Rust par fichier JS ; l'orchestrateur lui-même est réparti dans les fichiers core_*.rs.

pub mod admin;
pub mod agents;
pub mod ai_watch;
pub mod announce;
pub mod answer_cache;
pub mod app_index;
pub mod auto_perf;
pub mod automations;
pub mod briefing;
pub mod build_space;
pub mod browser_bridge;
pub mod cards;
pub mod checkup;
pub mod cli_daemon;
pub mod collections;
pub mod companion;
pub mod documents;
pub mod code_entry;
pub mod command_help;
pub mod config;
pub mod context;
pub mod context_mute;
pub mod desktop;
pub mod desktop_intents;
pub mod desktop_state;
pub mod devant;
pub mod drawing;
pub mod embeddings;
pub mod emotions;
pub mod failures;
pub mod file_search;
pub mod google;
pub mod google_docs;
pub mod hardware;
pub mod help;
pub mod history;
pub mod intent;
pub mod home;
pub mod parcels;
pub mod translate;
pub mod lights;
pub mod list_cards;
pub mod local_skills;
pub mod macros;
pub mod mail_filters;
pub mod mail_parse;
pub mod mail_watch;
pub mod mails;
pub mod mcp_client;
pub mod mcp_registry;
pub mod media;
pub mod meetings;
pub mod memory;
pub mod memory_extract;
pub mod micro_call;
pub mod missions;
pub mod notes;
pub mod policy;
pub mod prefilter;
pub mod presence;
pub mod projects;
pub mod pronunciation;
pub mod reminders;
pub mod research;
pub mod router;
pub mod savoir;
pub mod schedules;
pub mod security;
pub mod semantic_router;
pub mod shopping;
pub mod skill_intents;
pub mod skill_intents_more;
pub mod skills;
pub mod smart_lists;
pub mod speech_stream;
pub mod spotify;
pub mod status;
pub mod system;
pub mod telegram;
pub mod nomade;
pub mod reseau;
pub mod terminal_actions;
pub mod terminal_colors;
pub mod terminal_voice;
pub mod tmux;
pub mod tmux_voice;
pub mod ui_control;
pub mod undo;
pub mod usage_daily;
pub mod vibe_coding;
pub mod vision;
pub mod vocabulary;
pub mod voice_corpus;
pub mod voice_quality;
pub mod bg_hidden;
pub mod window_slots;
pub mod zones;

mod core_approvals;
mod core_commands;
mod core_http;
mod core_media;
mod core_misc;
mod core_start;
mod core_submit;
mod core_tasks;

pub use core_http::HttpReply;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::rc::{Rc, Weak};

use serde_json::{json, Value};

use crate::js::{self, Timer, J};
use crate::secret_box::SecretBox;

/// Futur local (non Send) : tout l'orchestrateur vit sur un seul fil (LocalSet).
pub type Fut<T> = Pin<Box<dyn Future<Output = T>>>;
/// Référence faible vers l'orchestrateur, détenue par chaque module.
pub type OrchRef = Weak<Orchestrator>;

pub trait Up {
    fn up(&self) -> Rc<Orchestrator>;
}

impl Up for OrchRef {
    fn up(&self) -> Rc<Orchestrator> {
        self.upgrade().expect("orchestrateur arrêté")
    }
}

/// Exécution d'un outil local derrière la politique de sécurité.
pub type ToolExec = Box<dyn FnOnce(J) -> Fut<J>>;

// --- Consignes et constantes ---------------------------------------------------------------------

/// Consigne permanente : les réponses sont lues à voix haute, pas affichées.
pub const PERSONA: &str = "Tu es Jarvis, l'assistant personnel vocal de l'utilisateur. Ta réponse finale est lue à voix haute par une synthèse vocale : réponds en français, sans markdown, sans liste, sans bloc de code, sans emoji. Quand une information est incomplète, ambiguë ou manque d'un paramètre essentiel (nom, dossier, choix), appelle impérativement l'outil demander_precision pour poser une question claire et concise à l'utilisateur. Quand l'utilisateur demande ce qui est affiché à l'écran, demande d'analyser une erreur ou de regarder son écran, appelle l'outil capture_ecran (avec avec_ocr: true) ou lire_ecran_ocr pour examiner l'image et le texte, puis réponds précisément. Par respect strict de la vie privée, ne capture jamais l'écran automatiquement : uniquement sur demande explicite de l'utilisateur. Quand l'utilisateur révèle une information personnelle durable (préférence, identité, habitude, projet en cours), appelle memoire_ajouter pour la retenir. Quand une demande dépend de ses goûts ou de son contexte, appelle memoire_lire avant de répondre. Une situation passagère mais utile plusieurs jours (congés, déplacement, invité) se retient avec expire_le, le dernier jour où elle reste vraie. N'enregistre jamais de mot de passe, de code de validation ni d'information sans lendemain. Ne mémorise jamais automatiquement dans la mémoire long terme le contenu des emails ou des rendez-vous personnels. Pour la musique en cours (lecture, pause, suivant, précédent, titre), utilise l'outil media du serveur system : le widget musique s'ouvre dans le HUD. Pour une demande de dessin (mouton, chat, maison…), utilise l'outil dessiner du serveur system : il ouvre le dessin dans Paint. Pour une question sur une réunion enregistrée (ce qui a été dit ou décidé, qui était là, quand), appelle reunions_chercher, puis reunion_lire si le texte complet est nécessaire ; cite la réunion et sa date. Pour une question sur ses listes de collection (vinyles, Lego, livres… : ce qu'il possède ou voudrait posséder, prix, lieu et date d'achat), appelle listes_chercher, puis listes_lire pour une fiche complète ; tu ne modifies jamais une liste. Pour une question sur un sujet que l'utilisateur a fait documenter (ses dossiers Data : un appareil, un produit, un thème), appelle d'abord savoir_chercher et appuie ta réponse sur ses extraits.";

/// Consignes système propres aux agents (Claude Code, AGY) pour créer, écrire et modifier des
/// fichiers : ajoutées après PERSONA. `{workspace}` = dossier de travail de Jarvis.
pub const AGENT_FILES_PROMPT: &str = "Consignes pour les fichiers (création, écriture, modification). \
1. Emplacement : un fichier demandé pour un projet se crée dans le dossier de ce projet, jamais dans ton dossier courant. \
Récupère son chemin avec l'outil projets du serveur system (action infos, champ path ou chemin). \
Projet non précisé ou ambigu : appelle demander_precision avant d'écrire. \
Fichier sans projet (note, brouillon, export) : dossier de travail de Jarvis, {workspace}. \
2. Chemins : toujours absolus, au format Windows (E:\\www\\…). Vérifie que le dossier visé existe (Glob) au lieu de deviner un chemin. \
3. Outils : pour créer, l'outil d'écriture (Write, ou write_to_file avec AGY), qui crée aussi les dossiers manquants. \
Pour modifier un fichier existant, lis-le d'abord (Read), puis Edit avec un extrait copié exactement du fichier et unique ; \
réécris-le en entier seulement s'il est court ou si on te le demande. \
N'écris, n'écrase ni ne modifie jamais de fichier par une commande (echo, redirection, Set-Content, Out-File, copy) : seuls les outils d'écriture et de modification le font. \
4. Contenu : complet et prêt à servir, jamais de « … » ni de passage à compléter ; respecte la langue, l'encodage UTF-8, l'indentation et les conventions du projet. \
Les règles de réponse orale (sans markdown, sans bloc de code, sans liste) ne s'appliquent qu'à ta réponse finale, pas au contenu des fichiers : \
Markdown, code et JSON y gardent leur format normal. \
5. Accord : chaque écriture hors du dossier de travail demande l'accord de l'utilisateur (« oui » à voix haute ou code) ; attends-le. \
Refus ou zone interdite : n'insiste pas, ne contourne pas par un autre outil, dis simplement ce qui n'a pas été fait. \
6. Vérification : contrôle le résultat de chaque outil. Edit échoue (extrait introuvable ou multiple) : relis le fichier et recommence avec un extrait exact. \
7. Réponse : une phrase qui dit quel fichier a été créé ou modifié et dans quel projet, sans recopier son contenu.";

/// Outils intégrés de Claude Code laissés au modèle (les outils MCP de Jarvis s'y ajoutent).
pub const CLAUDE_TOOLS: &str = "Bash,Read,Edit,Write,Glob,Grep,WebSearch,WebFetch";
/// Conversation renouvelée après 10 min sans échange ou 20 tours.
pub const SESSION_IDLE_MS: i64 = 10 * 60 * 1000;
pub const SESSION_MAX_TURNS: u32 = 20;
pub const APPROVAL_TIMEOUT: u64 = 180_000;

pub fn approval_port() -> u16 {
    js::env("JARVIS_APPROVAL_PORT").and_then(|p| p.parse().ok()).unwrap_or(8799)
}

/// Phrases courtes partagées avec le moteur vocal (voice/phrases.json).
pub fn phrases() -> &'static J {
    // Phrases courtes partagées avec le moteur vocal (cache audio) : voice/phrases.json de la v3.
    static P: std::sync::LazyLock<J> = std::sync::LazyLock::new(|| serde_json::from_str(include_str!("../../data/phrases.json")).unwrap_or(json!({})));
    &P
}

fn find_exe(env: &str, names: &[&str], extra: Vec<PathBuf>) -> String {
    if let Some(p) = js::env(env) {
        return p;
    }
    let mut candidates = extra;
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            for n in names {
                candidates.push(d.join(n));
            }
        }
    }
    candidates.into_iter().find(|p| p.exists()).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| names.last().unwrap_or(&"").to_string())
}

pub fn find_claude() -> String {
    find_exe("JARVIS_CLAUDE", &["claude.exe", "claude"], vec![js::home_dir().join(".local").join("bin").join("claude.exe")])
}

pub fn find_agy() -> String {
    find_exe("JARVIS_AGY", &["agy.exe", "agy"], vec![js::home_dir().join("AppData").join("Local").join("agy").join("bin").join("agy.exe")])
}

/// Le texte lu à voix haute reste une phrase, pas un bloc de code.
pub fn speakable(text: &str) -> String {
    let t = crate::re!(r"```[\s\S]*?```").replace_all(text, " ");
    let t = crate::re!(r"[`*#>_]").replace_all(&t, "");
    policy::shorten(t.trim(), 600)
}

/// Complexité d'une demande pour l'économie de quota : « simple » ou « complex ».
pub fn detect_task_complexity(text: &str) -> &'static str {
    let clean = js::fold_lower(text);
    if clean.chars().count() > 130 {
        return "complex";
    }
    let pats: [&regex::Regex; 5] = [
        crate::re!(r"\b(?:cree|creer|ecris|ecrire|developpe|developper|programme|programmer|code|coder)\b"),
        crate::re!(r"\b(?:refactor|refactore|refactorise|refactoriser|corrige|corriger|fix|debug|debogue|deboguer)\b"),
        crate::re!(r"\b(?:modifie|modifier|ajoute|ajouter|supprime|supprimer|remplace|remplacer|implemente|implementer)\b"),
        crate::re!(r"\b(?:analyse|analyser|compare|comparer|explique le code|trouve le bug|pourquoi.*(?:erreur|plante|marche pas))\b"),
        crate::re!(r"\b(?:test|tester|script|fonction|classe|composant|base de donnees|api|serveur|architecture)\b"),
    ];
    if pats.iter().any(|p| p.is_match(&clean)) { "complex" } else { "simple" }
}

/// Prénom donné pendant la première rencontre.
pub fn extract_name(input: &str) -> String {
    let clean = input.trim();
    if let Some(m) = crate::re!(r"(?i)(?:je m'appelle|moi c'est|c'est|mon prénom est|mon nom est|appelle-moi|on m'appelle)\s+([A-Za-zÀ-ÖØ-öø-ÿ'-]+)").captures(clean) {
        return m[1].trim().to_string();
    }
    let words: Vec<&str> = clean.split_whitespace().filter(|w| !crate::re!(r"(?i)^(bonjour|salut|hello|moi|je|suis|un|le|la|mon|nom|prenom|prénom|est|c'est)$").is_match(w)).collect();
    if let Some(w) = words.first().filter(|w| w.chars().count() >= 2) {
        return crate::re!(r"[.,!?;:]").replace_all(w, "").into_owned();
    }
    crate::re!(r"[.,!?;:]").replace_all(clean, "").trim().to_string()
}

pub fn extract_birthday(input: &str) -> String {
    let clean = input.trim();
    if let Some(m) = crate::re!(r"(?i)(\d{1,2})\s*(?:er)?\s+([a-zA-Zéûù]+)(?:\s+(\d{4}))?").captures(clean) {
        return format!("{} {}{}", &m[1], &m[2], m.get(3).map(|y| format!(" {}", y.as_str())).unwrap_or_default()).trim().to_string();
    }
    if let Some(m) = crate::re!(r"(\d{1,2})[/\-.](\d{1,2})(?:[/\-.](\d{2,4}))?").captures(clean) {
        let day: u32 = m[1].parse().unwrap_or(0);
        let mn: usize = m[2].parse().unwrap_or(0);
        if (1..=12).contains(&mn) {
            let year = m.get(3).map(|y| if y.as_str().len() == 2 { format!(" 19{}", y.as_str()) } else { format!(" {}", y.as_str()) }).unwrap_or_default();
            return format!("{day} {}{year}", js::MOIS[mn - 1]).trim().to_string();
        }
    }
    let c = crate::re!(r"(?i)^(?:je suis né le|né le|le|c'est le|mon anniversaire c'est le|mon anniversaire est le)\s+").replace(clean, "");
    crate::re!(r"[.,!?;:]").replace_all(&c, "").trim().to_string()
}

pub fn extract_city(input: &str) -> String {
    let clean = input.trim();
    if let Some(m) = crate::re!(r"(?i)(?:j'habite\s+à|j'habite\s+a|j'habite|je vis\s+à|je vis\s+a|je vis|ville\s*(?:de|:)?)\s+([A-Za-zÀ-ÖØ-öø-ÿ\s'-]+)").captures(clean) {
        return crate::re!(r"[.,!?;:]").replace_all(&m[1], "").trim().to_string();
    }
    if let Some(m) = crate::re!(r"(?i)^à\s+([A-Za-zÀ-ÖØ-öø-ÿ\s'-]+)").captures(clean) {
        return crate::re!(r"[.,!?;:]").replace_all(&m[1], "").trim().to_string();
    }
    let c = crate::re!(r"(?i)^(?:à|a|dans la ville de)\s+").replace(clean, "");
    crate::re!(r"[.,!?;:]").replace_all(&c, "").trim().to_string()
}

// --- État des tâches -----------------------------------------------------------------------------

/// Tâche IA en cours (processus Claude Code / AGY, ou démon résident).
pub struct RunningTask {
    pub text: String,
    pub engine: String,
    pub background: bool,
    pub started_at: i64,
    pub is_daemon: bool,
    pub pid: u32,
    pub child: Option<Rc<crate::proc::LineChild>>,
    pub task_mcp_path: Option<PathBuf>,
    pub meta: J,
}

/// Demande en attente d'un emplacement libre.
#[derive(Clone)]
pub struct QueuedTask {
    pub text: String,
    pub background: bool,
    /// 0 : urgent, 1 : interactif, 2 : tâche de fond
    pub priority: i64,
    pub queued_at: i64,
    pub speaker: J,
}

/// Appel en cours à une IA externe (cœur orange du HUD).
#[derive(Clone)]
pub struct AiCall {
    pub engine: String,
    pub model: String,
    pub started_at: i64,
    pub prompt_chars: usize,
    pub usage_seen: bool,
    pub background: bool,
}

/// Contrôle d'une validation en attente : réponse à rendre à l'outil MCP et minuteur d'expiration.
pub struct PendingCtl {
    pub resolve: Option<tokio::sync::oneshot::Sender<J>>,
    pub timer: Timer,
    pub on_decision: Option<Box<dyn FnOnce(bool)>>,
}

/// Question de précision posée par l'IA (outil demander_precision), en attente de la réponse.
pub struct WaitingReply {
    pub task_id: String,
    pub question: String,
    pub tx: Option<tokio::sync::oneshot::Sender<J>>,
    pub timer: Timer,
}

/// Paramètres de construction.
pub struct OrchOptions {
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub models_dir: Option<PathBuf>,
    pub emit: Rc<dyn Fn(J)>,
    pub log: Rc<dyn Fn(String)>,
    pub secret_box: Option<Rc<SecretBox>>,
    pub python: Option<String>,
    /// Configuration MCP d'AGY à tenir à jour (seul le vrai Jarvis la fournit).
    pub agy_config_path: Option<PathBuf>,
    pub notes_dir: Option<PathBuf>,
    pub meetings_dir: Option<PathBuf>,
}

pub struct Orchestrator {
    me: Weak<Orchestrator>,
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub mcp_config: PathBuf,
    pub agy_config_path: Option<PathBuf>,
    raw_emit: Rc<dyn Fn(J)>,
    log_fn: Rc<dyn Fn(String)>,
    pub emit_listeners: RefCell<Vec<Rc<dyn Fn(&J)>>>,
    pub notify_listeners: RefCell<Vec<Rc<dyn Fn(&J)>>>,
    pub secret_box: Option<Rc<SecretBox>>,
    pub python: Option<String>,
    pub claude: String,
    pub agy: String,

    // Modules
    pub memory: memory::Memory,
    pub embedder: Option<Rc<embeddings::Embedder>>,
    pub emotions: emotions::Emotions,
    pub history: history::History,
    pub status: status::Status,
    pub usage_daily: usage_daily::UsageDaily,
    pub config: config::Config,
    pub security: security::Security,
    pub mcp_registry: mcp_registry::McpRegistry,
    pub media: Rc<media::MediaBridge>,
    pub context_mute: Rc<context_mute::ContextMute>,
    pub zones: zones::Zones,
    pub app_index: app_index::AppIndex,
    pub vocabulary: vocabulary::Vocabulary,
    pub google: Rc<google::GoogleConnector>,
    pub spotify: spotify::Spotify,
    pub file_search: file_search::FileSearch,
    pub desktop: desktop::Desktop,
    pub tmux: Rc<tmux::Tmux>,
    pub tmux_voice: Rc<tmux_voice::TmuxVoice>,
    pub terminal_actions: terminal_actions::TerminalActions,
    pub terminal_voice: Rc<terminal_voice::TerminalVoice>,
    pub pronunciation: pronunciation::Pronunciation,
    pub cards: cards::Cards,
    pub projects: Rc<projects::Projects>,
    pub command_help: command_help::CommandHelp,
    pub reminders: Rc<reminders::Reminders>,
    pub schedules: schedules::Schedules,
    pub briefing: briefing::Briefing,
    pub mail_watch: mail_watch::MailWatch,
    /// Compagnon de bureau (tamagotchi), option de l'administration.
    pub companion: companion::Companion,
    pub ai_watch: Rc<ai_watch::AiWatch>,
    pub terminal_colors: Rc<terminal_colors::TerminalColors>,
    pub list_cards: list_cards::ListCards,
    pub mails: mails::MailReader,
    pub mail_filters: mail_filters::MailFilters,
    pub undo: undo::UndoStack,
    pub context: context::ActiveContext,
    pub research: research::Research,
    pub shopping: shopping::Shopping,
    pub smart_lists: smart_lists::SmartLists,
    pub collections: collections::Collections,
    pub documents: documents::Documents,
    pub savoir: savoir::Savoir,
    pub skills: Rc<skills::Skills>,
    pub lights: lights::Lights,
    pub home: home::Home,
    pub parcels: parcels::Parcels,
    pub memory_extract: memory_extract::MemoryExtract,
    pub announcer: announce::Announcer,
    pub presence: presence::Presence,
    pub missions: missions::Missions,
    pub vision: vision::Vision,
    pub vibe_coding: vibe_coding::VibeCoding,
    pub desktop_state: Rc<desktop_state::DesktopState>,
    pub devant: devant::Devant,
    pub window_slots: window_slots::WindowSlots,
    pub bg_hidden: bg_hidden::BgHidden,
    pub browser_bridge: Rc<browser_bridge::BrowserBridge>,
    pub ui_control: ui_control::UiControl,
    pub code_entry: code_entry::CodeEntry,
    pub automations: Rc<automations::Automations>,
    pub agents: Rc<agents::Agents>,
    pub notes: notes::Notes,
    pub meetings: Rc<meetings::Meetings>,
    pub google_docs: Rc<google_docs::GoogleDocs>,
    pub telegram: Rc<telegram::Telegram>,
    pub nomade: Rc<nomade::Nomade>,
    /// Jarvis Réseau : amis, partages, messages chiffrés de bout en bout (reseau.rs).
    pub reseau: Rc<reseau::Reseau>,
    pub failures: failures::Failures,
    pub macros: macros::Macros,
    pub answer_cache: answer_cache::AnswerCache,
    pub semantic_router: Rc<semantic_router::SemanticRouter>,
    pub router: router::Router,
    pub cli_daemon: Rc<cli_daemon::CliDaemon>,
    pub hardware_probe: RefCell<Option<Rc<hardware::HardwareProbe>>>,
    pub auto_perf: RefCell<Option<auto_perf::AutoPerf>>,

    // État des demandes et des tâches
    pub ai_calls: RefCell<HashMap<String, AiCall>>,
    pub tool_uses: RefCell<HashMap<String, J>>,
    pub last_vocabulary: RefCell<Option<String>>,
    pub last_file_results: RefCell<Option<(Vec<String>, i64)>>,
    pub dictation: RefCell<Option<J>>,
    pub pending_ai_confirm: RefCell<Option<(String, J, i64)>>,
    pub speculative: RefCell<Option<(String, intent::Intent, i64)>>,
    pub tool_cache: RefCell<HashMap<String, (J, i64)>>,
    pub last_spoken: RefCell<String>,
    /// Phrases dites récemment (texte, heure en ms) : une même phrase n'est pas redite avant une minute, ni plus de deux fois.
    say_seen: RefCell<Vec<(String, i64)>>,
    /// Heure du dernier accusé de réception (« je m'en occupe… ») : un seul toutes les 10 secondes, quelle que soit la formule.
    ack_at: Cell<i64>,
    pub session_id: RefCell<Option<String>>,
    pub session_turns: Cell<u32>,
    pub session_last_at: Cell<i64>,
    /// Validations en attente : (tâche, informations : outil, niveau, code, saisie…), ordre d'arrivée.
    pub pending: RefCell<Vec<(String, J)>>,
    pub pending_ctl: RefCell<HashMap<String, PendingCtl>>,
    pub pre_approved: RefCell<HashMap<String, i64>>,
    pub queue: RefCell<VecDeque<QueuedTask>>,
    pub running: RefCell<Vec<(String, RunningTask)>>,
    pub running_task: RefCell<Option<String>>,
    pub max_concurrent: usize,
    pub last_answer: RefCell<Option<J>>,
    pub waiting_reply: RefCell<Option<WaitingReply>>,
    pub onboarding: RefCell<Option<J>>,
    pub counter: Cell<u64>,
    /// Délai (ms) entre le lancement d'une tâche IA et son premier mot, les 30 dernières (Checkup).
    pub first_word_ms: RefCell<Vec<i64>>,
    pub speech: RefCell<HashMap<String, Rc<speech_stream::SpeechStream>>>,
    pub session_epoch: Cell<u64>,
    pub last_ack: RefCell<Option<(String, i64)>>,
    pub started: Cell<bool>,
    pub stopped: Cell<bool>,
    pub current_speaker: RefCell<Option<String>>,
    pub last_request_source: RefCell<String>,
    pub last_widget_request: RefCell<Option<(String, i64)>>,
    /// Demandes venues d'une card (Actions, lignes…) : instant de la dernière réponse dite, par phrase (voir `submit_inner`).
    pub widget_announced: RefCell<HashMap<String, i64>>,
    /// Vrai pendant le traitement d'une demande de card répétée en moins de 20 s : sa réponse s'affiche mais n'est pas dite.
    pub quiet_say: Cell<bool>,
    /// Dernière musique mémorisée pour « mets de la musique sur Spotify » (titre + artiste).
    pub spotify_last: RefCell<String>,
    pub last_voice_utterance: RefCell<Option<(String, i64)>>,
    pub waiting_vocab_clarification: RefCell<Option<J>>,
    pub semantic_disabled: Cell<bool>,
    pub last_desktop_card: RefCell<Option<J>>,
    /// Fenêtre que le dernier clic « basculer » d'une card a montrée (le clic suivant la réduit).
    pub toggle_shown: Cell<i64>,
    pub desktop_card_sig: RefCell<String>,
    pub engine_errors: RefCell<HashMap<String, (String, i64)>>,
    pub agy_blocked_until: Cell<i64>,
    pub agy_fallback_announced: Cell<bool>,
    /// Dernier micro-appel (question, réponse, instant) : contexte de la relance suivante.
    pub last_micro: RefCell<Option<(String, String, i64)>>,
    /// État du lecteur interne (titre, artiste, pochette…), envoyé par l'interface.
    pub internal_music_state: RefCell<J>,
    /// Dernière ouverture automatique de l'administration, par connecteur (requestSetup).
    pub setup_opened_at: RefCell<HashMap<String, i64>>,
    pub silence_restore_mode: RefCell<Option<String>>,
    /// Le moteur vocal est-il prêt ? « ok », « wait » ou « off » (fourni par le noyau).
    pub voice_probe: RefCell<Option<Rc<dyn Fn() -> String>>>,
    /// Réglages d'optimisation effectifs changés par le mode automatique (journal de mesure du noyau).
    pub on_auto_perf_applied: RefCell<Option<Rc<dyn Fn(&J)>>>,
    /// Statut du moteur vocal (empreintes vocales…), fourni par la liaison vocale.
    pub speaker_status_source: RefCell<Option<Rc<dyn Fn() -> J>>>,

    // Minuteurs
    pub telemetry_timer: Timer,
    pub audio_timer: Timer,
    pub index_timer: Timer,
    /// Compilations (target/) : dernière mesure, mesure ou nettoyage en cours, contrôle périodique.
    pub build_space: RefCell<J>,
    pub build_space_busy: Cell<bool>,
    pub build_space_timer: Timer,
    pub tmux_watch_timer: Timer,
    pub claude_watch_timer: Timer,
    pub desktop_refresh_timer: Timer,
    pub projects_memory_timer: Timer,
    pub agy_unblock_timer: Timer,
    pub last_overload_log: Cell<i64>,
    /// Dernière télémétrie de la machine (CPU, RAM), lue par la card Checkup.
    pub last_telemetry: RefCell<J>,
    pub checkup_timer: Timer,
    pub audio_probed_once: Cell<bool>,
    pub http_server: RefCell<Option<tokio::task::JoinHandle<()>>>,
}

/// Délai entre deux accusés de réception.
const SAY_REPEAT_MS: i64 = 10_000;
/// Délai pendant lequel une phrase identique n'est pas répétée.
const SAY_SAME_MS: i64 = 60_000;
/// Fenêtre où une phrase identique est dite au plus `SAY_MAX_TIMES` fois (état d'un terminal…).
const SAY_COUNT_MS: i64 = 30 * 60_000;
const SAY_MAX_TIMES: usize = 2;

/// Clé de comparaison d'une phrase : casse et espaces ignorés.
fn say_key(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Vrai si la phrase (clé `key`) ne doit pas être redite : dite il y a moins d'une minute, ou (si `capped`)
/// déjà dite deux fois dans la demi-heure. Purge l'historique au passage.
fn say_blocked(seen: &mut Vec<(String, i64)>, key: &str, now: i64, capped: bool) -> bool {
    seen.retain(|(_, t)| now.saturating_sub(*t) < SAY_COUNT_MS);
    let same: Vec<i64> = seen.iter().filter(|(s, _)| s == key).map(|(_, t)| *t).collect();
    same.iter().any(|t| now.saturating_sub(*t) < SAY_SAME_MS) || (capped && same.len() >= SAY_MAX_TIMES)
}

impl Orchestrator {
    pub fn new(opts: OrchOptions) -> Rc<Orchestrator> {
        let d = opts.data_dir.clone();
        let log = opts.log.clone();
        let sbox = opts.secret_box.clone();
        let models_dir = opts.models_dir.clone().or_else(|| js::env("JARVIS_MODELS_DIR").map(PathBuf::from)).unwrap_or_else(|| d.join("models"));
        Rc::new_cyclic(|w: &Weak<Orchestrator>| {
            let w = w.clone();
            let lg = log.clone();
            let emit_w = w.clone();
            let emit: Rc<dyn Fn(J)> = Rc::new(move |e| {
                if let Some(o) = emit_w.upgrade() {
                    o.emit(e);
                }
            });
            let notify_w = w.clone();
            let notify: Rc<dyn Fn(J)> = Rc::new(move |n| {
                if let Some(o) = notify_w.upgrade() {
                    o.notify(n);
                }
            });
            let embedder = embeddings::Embedder::new(models_dir, lg.clone());
            let memory = memory::Memory::new(d.join("memory.json"));
            memory.attach_embedder(Some(embedder.clone()));
            let tmux_w = w.clone();
            let tmux = Rc::new(tmux::Tmux::new(Box::new(move || tmux_w.upgrade().map(|o| o.config.get("tmux")).unwrap_or(json!({}))), lg.clone()));
            let projects_w = w.clone();
            let projects_w2 = w.clone();
            let pron_w = w.clone();
            let meet_w = w.clone();
            let meet_w2 = w.clone();
            let mf_w = w.clone();
            let media_w = w.clone();
            let gd_open: Rc<dyn Fn(String)> = Rc::new(|url| {
                js::spawn(async move {
                    let _ = system::ouvrir_url(&json!({ "url": url })).await;
                });
            });
            let stderr_w = w.clone();
            let negative = regex::Regex::new(failures::NEGATIVE_REPLY).unwrap();
            let semantic_router = semantic_router::SemanticRouter::new(embedder.clone(), Some(d.join("runtime").join("intent-vectors.json")), lg.clone());
            Orchestrator {
                me: w.clone(),
                workspace: opts.workspace.clone(),
                data_dir: d.clone(),
                runtime_dir: d.join("runtime"),
                mcp_config: d.join("runtime").join("mcp.json"),
                agy_config_path: opts.agy_config_path.clone(),
                raw_emit: opts.emit.clone(),
                log_fn: lg.clone(),
                emit_listeners: RefCell::new(vec![]),
                notify_listeners: RefCell::new(vec![]),
                secret_box: sbox.clone(),
                python: opts.python.clone(),
                claude: find_claude(),
                agy: find_agy(),
                memory,
                embedder: Some(embedder.clone()),
                emotions: emotions::Emotions::new(d.join("emotions.json"), lg.clone()),
                history: history::History::new(d.join("history").join("journal.jsonl"), lg.clone(), true),
                status: status::Status::new(d.join("session.json")),
                usage_daily: usage_daily::UsageDaily::new(d.join("usage-daily.json")),
                config: config::Config::new(d.join("config.json")),
                security: security::Security::new(d.join("security.json")),
                mcp_registry: mcp_registry::McpRegistry::new(d.join("mcp-servers.json"), sbox.clone()),
                media: media::MediaBridge::new(lg.clone(), Rc::new(move |state: J| {
                    if let Some(o) = media_w.upgrade() {
                        let mut ev = json!({ "type": "media" });
                        js::assign(&mut ev, state);
                        o.emit(ev);
                    }
                })),
                context_mute: context_mute::ContextMute::new(
                    lg.clone(),
                    emit.clone(),
                    Some(d.join("session.json")),
                    Box::new(move || meet_w.upgrade().and_then(|o| o.meetings_tab())),
                    Box::new(move || meet_w2.upgrade().map(|o| o.config.get("visio")["meetSilence"] != json!(false)).unwrap_or(true)),
                ),
                zones: zones::Zones::new(d.join("zones.json"), opts.workspace.clone()),
                app_index: app_index::AppIndex::new(lg.clone()),
                vocabulary: vocabulary::Vocabulary::new(d.join("vocabulary.json"), w.clone()),
                google: Rc::new(google::GoogleConnector::new(d.join("google.json"), d.join("calendar.json"), sbox.clone())),
                spotify: spotify::Spotify::new(d.join("spotify.json"), sbox.clone(), lg.clone()),
                file_search: file_search::FileSearch::new(w.clone(), lg.clone()),
                desktop: desktop::Desktop::new(opts.python.clone()),
                tmux_voice: tmux_voice::TmuxVoice::new(w.clone(), tmux.clone()),
                tmux,
                terminal_actions: terminal_actions::TerminalActions::new(d.join("terminal-actions.json"), lg.clone()),
                terminal_voice: terminal_voice::TerminalVoice::new(w.clone()),
                pronunciation: pronunciation::Pronunciation::new(d.join("prononciation.json"), Box::new(move || pron_w.upgrade().map(|o| o.projects.list().iter().map(|p| js::gs(p, "name")).collect()).unwrap_or_default()), lg.clone()),
                cards: cards::Cards::new(w.clone()),
                projects: Rc::new(projects::Projects::new(
                    Box::new(move || projects_w.upgrade().map(|o| js::gs(&o.config.get("projects"), "root")).unwrap_or_default()),
                    Some(d.join("projets.json")),
                    Box::new(move || {
                        if let Some(o) = projects_w2.upgrade() {
                            o.sync_projects_memory();
                        }
                    }),
                    lg.clone(),
                )),
                command_help: command_help::CommandHelp::new(w.clone()),
                reminders: reminders::Reminders::new(d.join("rappels.json"), notify.clone(), lg.clone()),
                schedules: schedules::Schedules::new(w.clone(), d.join("programmations.json"), lg.clone()),
                briefing: briefing::Briefing::new(w.clone(), d.join("briefing.json")),
                mail_watch: mail_watch::MailWatch::new(w.clone(), d.join("mails-vus.json"), lg.clone()),
                companion: companion::Companion::new(w.clone(), d.join("companion.json"), lg.clone()),
                ai_watch: ai_watch::AiWatch::new(w.clone(), lg.clone()),
                terminal_colors: terminal_colors::TerminalColors::new(w.clone(), d.join("couleurs-terminaux.json"), lg.clone()),
                list_cards: list_cards::ListCards::new(w.clone()),
                mails: mails::MailReader::new(w.clone(), lg.clone()),
                mail_filters: mail_filters::MailFilters::new(
                    d.join("mails-exclus.json"),
                    w.clone(),
                    Box::new(move |op: &str, entry: &J| {
                        if let Some(o) = mf_w.upgrade() {
                            o.publish_memory();
                            o.record(if op == "add" { "memoire_ajout" } else { "memoire_oubli" }, json!({ "categorie": entry["category"], "texte": entry["text"] }));
                            o.refresh_daemon();
                            if op == "add" {
                                let o2 = o.clone();
                                js::spawn(async move {
                                    let _ = o2.memory.index().await;
                                });
                            }
                        }
                    }),
                    lg.clone(),
                ),
                undo: undo::UndoStack::new(w.clone()),
                context: context::ActiveContext::new(w.clone()),
                research: research::Research::new(w.clone()),
                shopping: shopping::Shopping::new(w.clone(), Some(d.join("produits.json")), lg.clone()),
                smart_lists: smart_lists::SmartLists::new(w.clone(), d.join("lists"), lg.clone()),
                collections: collections::Collections::new(w.clone(), d.join("collections"), lg.clone()),
                documents: documents::Documents::new(w.clone(), d.join("documents"), lg.clone()),
                savoir: savoir::Savoir::new(w.clone(), d.join("savoir"), lg.clone()),
                skills: skills::Skills::new(w.clone(), crate::paths::root().join("skills"), lg.clone()),
                lights: lights::Lights::new(d.join("lumieres.json"), lg.clone()),
                home: home::Home::new(d.join("maison.json"), lg.clone()),
                parcels: parcels::Parcels::new(w.clone(), d.join("colis.json"), lg.clone()),
                memory_extract: memory_extract::MemoryExtract::new(w.clone(), d.join("memoire-auto.json"), lg.clone()),
                announcer: announce::Announcer::new(w.clone()),
                presence: presence::Presence::new(w.clone()),
                missions: missions::Missions::new(w.clone(), d.join("mission.json"), Rc::new(|s: &str| router::fold(s)), negative, Rc::new(|a: &str| failures::incapable(a)), lg.clone()),
                vision: vision::Vision::new(w.clone(), None),
                vibe_coding: vibe_coding::VibeCoding::new(w.clone(), lg.clone(), opts.workspace.clone(), None),
                desktop_state: desktop_state::DesktopState::new(lg.clone()),
                devant: devant::Devant::new(w.clone(), d.join("devant-exclus.json"), lg.clone()),
                window_slots: window_slots::WindowSlots::new(d.join("emplacements.json"), lg.clone()),
                bg_hidden: bg_hidden::BgHidden::new(d.join("bureau-masques.json"), lg.clone()),
                browser_bridge: browser_bridge::BrowserBridge::new(lg.clone()),
                ui_control: ui_control::UiControl::new(w.clone(), Some(d.join("applis-confiance.json")), lg.clone()),
                code_entry: code_entry::CodeEntry::new(w.clone()),
                automations: automations::Automations::new(w.clone(), d.join("automations.json"), lg.clone()),
                agents: agents::Agents::new(w.clone(), d.join("agents.json"), lg.clone()),
                notes: notes::Notes::new(w.clone(), opts.notes_dir.clone().or_else(|| js::env("JARVIS_NOTES_DIR").map(PathBuf::from)).unwrap_or_else(|| d.join("notes")), lg.clone()),
                meetings: meetings::Meetings::new(w.clone(), opts.meetings_dir.clone().or_else(|| js::env("JARVIS_MEETINGS_DIR").map(PathBuf::from)).unwrap_or_else(|| d.join("meetings")), lg.clone(), true),
                google_docs: google_docs::GoogleDocs::new(d.join("google-docs.json"), sbox.clone(), gd_open, lg.clone()),
                telegram: telegram::Telegram::new(w.clone(), d.join("telegram.json"), sbox.clone(), lg.clone()),
                nomade: nomade::Nomade::new(w.clone(), d.join("nomade.json"), d.join("nomade"), sbox.clone(), lg.clone()),
            reseau: reseau::Reseau::new(w.clone(), d.join("reseau.json"), d.join("reseau"), sbox.clone(), lg.clone()),
                failures: failures::Failures::new(d.join("echecs.json"), lg.clone()),
                macros: macros::Macros::new(d.join("macros.json")),
                answer_cache: answer_cache::AnswerCache::default(),
                semantic_router,
                router: router::Router::new(w.clone(), lg.clone()),
                cli_daemon: cli_daemon::CliDaemon::new(opts.workspace.clone(), lg.clone(), Rc::new(move |engine: &str, text: &str| {
                    if let Some(o) = stderr_w.upgrade() {
                        o.note_engine_error(engine, text);
                    }
                })),
                hardware_probe: RefCell::new(None),
                auto_perf: RefCell::new(None),
                ai_calls: RefCell::new(HashMap::new()),
                tool_uses: RefCell::new(HashMap::new()),
                last_vocabulary: RefCell::new(None),
                last_file_results: RefCell::new(None),
                dictation: RefCell::new(None),
                pending_ai_confirm: RefCell::new(None),
                speculative: RefCell::new(None),
                tool_cache: RefCell::new(HashMap::new()),
                last_spoken: RefCell::new(String::new()),
                say_seen: RefCell::new(Vec::new()),
                ack_at: Cell::new(0),
                session_id: RefCell::new(None),
                session_turns: Cell::new(0),
                session_last_at: Cell::new(0),
                pending: RefCell::new(vec![]),
                pending_ctl: RefCell::new(HashMap::new()),
                pre_approved: RefCell::new(HashMap::new()),
                queue: RefCell::new(VecDeque::new()),
                running: RefCell::new(vec![]),
                running_task: RefCell::new(None),
                max_concurrent: js::env("JARVIS_MAX_TASKS").and_then(|n| n.parse().ok()).filter(|n: &usize| *n >= 1).unwrap_or(4),
                last_answer: RefCell::new(None),
                waiting_reply: RefCell::new(None),
                onboarding: RefCell::new(None),
                counter: Cell::new(0),
                first_word_ms: RefCell::new(Vec::new()),
                speech: RefCell::new(HashMap::new()),
                session_epoch: Cell::new(0),
                last_ack: RefCell::new(None),
                started: Cell::new(false),
                stopped: Cell::new(false),
                current_speaker: RefCell::new(None),
                last_request_source: RefCell::new("voice".into()),
                last_widget_request: RefCell::new(None),
                widget_announced: RefCell::new(HashMap::new()),
                quiet_say: Cell::new(false),
                spotify_last: RefCell::new(String::new()),
                last_voice_utterance: RefCell::new(None),
                waiting_vocab_clarification: RefCell::new(None),
                semantic_disabled: Cell::new(false),
                last_desktop_card: RefCell::new(None),
                toggle_shown: Cell::new(0),
                desktop_card_sig: RefCell::new(String::new()),
                engine_errors: RefCell::new(HashMap::new()),
                agy_blocked_until: Cell::new(0),
                agy_fallback_announced: Cell::new(false),
                last_micro: RefCell::new(None),
                internal_music_state: RefCell::new(json!({})),
                setup_opened_at: RefCell::new(HashMap::new()),
                silence_restore_mode: RefCell::new(None),
                voice_probe: RefCell::new(None),
                on_auto_perf_applied: RefCell::new(None),
                speaker_status_source: RefCell::new(None),
                telemetry_timer: Timer::new(),
                audio_timer: Timer::new(),
                index_timer: Timer::new(),
                build_space: RefCell::new(J::Null),
                build_space_busy: Cell::new(false),
                build_space_timer: Timer::new(),
                tmux_watch_timer: Timer::new(),
                claude_watch_timer: Timer::new(),
                desktop_refresh_timer: Timer::new(),
                projects_memory_timer: Timer::new(),
                agy_unblock_timer: Timer::new(),
                last_overload_log: Cell::new(0),
                last_telemetry: RefCell::new(J::Null),
                checkup_timer: Timer::new(),
                audio_probed_once: Cell::new(false),
                http_server: RefCell::new(None),
            }
        })
        .post_init()
    }

    /// Réglages faits une fois l'orchestrateur construit (les modules peuvent alors le joindre).
    fn post_init(self: Rc<Self>) -> Rc<Self> {
        self.security.set_extra(self.mcp_registry.security_entries());
        let me = Rc::downgrade(&self);
        *self.failures.on_change.borrow_mut() = Some(Rc::new(move |counts: &J| {
            if let Some(o) = me.upgrade() {
                let mut ev = json!({ "type": "failures" });
                js::assign(&mut ev, counts.clone());
                o.emit(ev);
            }
        }));
        self.vibe_coding.load_verbosite();
        self.collections.index.attach_embedder(self.embedder.clone());
        self.documents.index.attach_embedder(self.embedder.clone());
        self.savoir.attach_embedder(self.embedder.clone());
        self.parcels.start();
        self.register_card_handlers();
        system::LIVE_DESKTOP.with(|l| *l.borrow_mut() = Rc::downgrade(&self.desktop_state));
        self
    }

    /// Cards d'application : volet psmux (tmux) et terminal (fenêtre).
    fn register_card_handlers(&self) {
        let w = self.me.clone();
        let (w1, w2, w3, w4) = (w.clone(), w.clone(), w.clone(), w.clone());
        self.cards.handlers.borrow_mut().insert(
            "tmux".into(),
            Rc::new(cards::Handler {
                locked: true,
                open: Some(Rc::new(move |app: J| {
                    let o = w1.up();
                    Box::pin(async move { o.tmux_voice.show_pane(&app).await })
                })),
                close: Some(Rc::new(move |app: J| {
                    let o = w2.up();
                    Box::pin(async move { o.tmux_voice.close_pane(&app).await })
                })),
                alive: Some(Rc::new(move |app: J| {
                    let o = w3.up();
                    Box::pin(async move { o.tmux_voice.card_alive(&app).await })
                })),
                action: Some(Rc::new(move |app: J, action: String| {
                    let o = w4.up();
                    Box::pin(async move { o.tmux_voice.card_action(&app, &action).await })
                })),
            }),
        );
        let (w1, w2, w3, w4) = (w.clone(), w.clone(), w.clone(), w.clone());
        // Card « Terminal » (agents Claude Code / AGY lancés) : le ✕ retire la card, il ne ferme pas le terminal.
        self.cards.handlers.borrow_mut().insert(
            "terminal_ai".into(),
            Rc::new(cards::Handler {
                locked: false,
                open: Some(Rc::new(move |app: J| {
                    let o = w1.up();
                    Box::pin(async move {
                        match app["hwnd"].as_i64() {
                            Some(h) if h != 0 => o.desktop_action(h, "afficher").await,
                            _ => json!({ "ok": false, "say": "Aucune fenêtre de terminal à afficher." }),
                        }
                    })
                })),
                close: Some(Rc::new(move |_app: J| {
                    let o = w2.up();
                    Box::pin(async move {
                        o.terminal_voice.dismiss_card();
                        json!({ "ok": true })
                    })
                })),
                alive: Some(Rc::new(move |app: J| {
                    let o = w3.up();
                    Box::pin(async move { o.terminal_voice.alive(&app) })
                })),
                action: Some(Rc::new(move |app: J, action: String| {
                    let o = w4.up();
                    Box::pin(async move { o.terminal_voice.card_action(&app, &action).await })
                })),
            }),
        );
    }

    pub fn self_ref(&self) -> OrchRef {
        self.me.clone()
    }

    pub fn log(&self, m: String) {
        (self.log_fn)(m)
    }

    /// Diffuse un événement vers l'interface (et les abonnés : canal Telegram…).
    pub fn emit(&self, mut event: J) {
        // Demande menée à bien ou en échec : l'humeur du compagnon de bureau s'en ressent (s'il est activé).
        if js::gs(&event, "type") == "finish" {
            self.companion.on_task_finished(js::gs(&event, "status") != "error");
        }
        // Musique en lecture : la sourdine contextuelle coupe la voix de Jarvis (réglage audio.musicSilence, actif par défaut).
        if js::gs(&event, "type") == "media" {
            let playing = js::gb(&event, "active") && js::gs(&event, "status") == "playing";
            self.context_mute.set_music(playing, self.config.get("audio")["musicSilence"] != json!(false));
            self.remember_spotify_track(&event);
        }
        if js::gs(&event, "type") == "say" && js::truthy(&event["text"]) {
            let (ack, stream, answer) = (js::gb(&event, "ack"), js::gb(&event, "stream"), js::gb(&event, "answer"));
            // Même phrase déjà dite il y a moins de 10 s (ex. 18 recherches d'images lancées d'un coup) : on ne la répète pas.
            if ack {
                let now = js::now_ms();
                if now - self.ack_at.get() < SAY_REPEAT_MS {
                    return;
                }
                self.ack_at.set(now);
            }
            // Phrase identique : pas avant une minute ; une annonce spontanée (état d'un terminal…) deux fois au plus
            // en une demi-heure. Annonce bloquée : abandonnée ; réponse à une demande : affichée sans être redite ;
            // « répète » : toujours dit.
            if !stream && !ack && !js::gb(&event, "force") {
                let now = js::now_ms();
                let key = say_key(&js::gs(&event, "text"));
                let mut seen = self.say_seen.borrow_mut();
                if say_blocked(&mut seen, &key, now, !answer) {
                    if !answer {
                        return;
                    }
                    event["muted"] = json!(true);
                } else {
                    seen.push((key, now));
                }
            }
            if !ack && !stream {
                *self.last_spoken.borrow_mut() = js::gs(&event, "text");
            }
            // Dialogue complet au journal : toute parole de Jarvis qui n'y figure pas déjà.
            if !ack && !stream && !answer {
                let mut data = json!({ "texte": js::slice_chars(&js::gs(&event, "text"), 2000) });
                if js::truthy(&event["task"]) {
                    data["tache"] = event["task"].clone();
                }
                self.record("parole", data);
            }
            if self.quiet_say.get() {
                event["muted"] = json!(true); // demande de card répétée : affichée, pas redite
            }
            if self.context_mute.is_muted.get() {
                event["muted"] = json!(true);
                event["muteReason"] = json!(*self.context_mute.reason.borrow());
            }
        }
        (self.raw_emit)(event.clone());
        let listeners = self.emit_listeners.borrow().clone();
        for f in listeners {
            f(&event);
        }
    }

    /// Onglet Google Meet ouvert (sourdine contextuelle), sans attendre la construction des réunions.
    fn meetings_tab(&self) -> Option<J> {
        None.or_else(|| self.meetings.first_meet_tab())
    }

    pub fn next_id(&self, prefix: &str) -> String {
        self.counter.set(self.counter.get() + 1);
        format!("{prefix}{}", self.counter.get())
    }
}

#[allow(unused)]
fn _v(_: Value) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn say(seen: &mut Vec<(String, i64)>, text: &str, now: i64, capped: bool) -> bool {
        let key = say_key(text);
        let blocked = say_blocked(seen, &key, now, capped);
        if !blocked {
            seen.push((key, now));
        }
        !blocked
    }

    #[test]
    fn meme_phrase_pas_avant_une_minute_ni_trois_fois() {
        let mut seen = Vec::new();
        let t = "Diagnostic terminé. Tous les systèmes sont opérationnels.";
        assert!(say(&mut seen, t, 0, true));
        assert!(!say(&mut seen, "diagnostic  terminé. tous les systèmes sont opérationnels.", 30_000, true));
        assert!(say(&mut seen, "Autre phrase", 30_000, true));
        assert!(say(&mut seen, t, 61_000, true));
        assert!(!say(&mut seen, t, 200_000, true)); // troisième fois : non
        assert!(say(&mut seen, t, 200_000 + SAY_COUNT_MS, true)); // demi-heure écoulée
    }

    #[test]
    fn reponse_demandee_limitee_a_une_minute() {
        let mut seen = Vec::new();
        assert!(say(&mut seen, "C'est fait.", 0, false));
        assert!(!say(&mut seen, "C'est fait.", 10_000, false));
        assert!(say(&mut seen, "C'est fait.", 70_000, false));
        assert!(say(&mut seen, "C'est fait.", 140_000, false));
    }
}
