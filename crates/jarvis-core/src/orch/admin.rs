// Service de la page d'administration (orchestrator/admin.js de la v3).
//
// Seule la fenêtre d'administration l'appelle (la passerelle vérifie le rôle) : ni Claude Code, ni
// AGY, ni la voix n'y ont accès. Chaque modification est inscrite au journal scellé. Les actions
// destructrices ou qui abaissent la sécurité exigent un code à 4 chiffres, affiché puis retapé.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{json, Map, Value};

use super::{memory, status, Fut, Orchestrator};
use crate::js::{self, J};
use crate::voice_supervisor::VoiceSupervisor;

const CHALLENGE_TTL: i64 = 120000;

const THEMES: &[(&str, &str)] = &[
    ("cyan", "Cyan Hologramme"),
    ("gold", "Stark Ambre & Or"),
    ("matrix", "Émeraude Matrix"),
    ("crimson", "Rubis Crimson"),
    ("violet", "Synthwave Améthyste"),
];
const DENSITIES: &[(&str, &str)] = &[("compact", "Compacte"), ("normal", "Normale"), ("large", "Large")];
const LISTENING: &[(&str, &str)] = &[("wakeword", "Mot d’éveil (« Jarvis »)"), ("ptt", "Appui pour parler (Espace)"), ("free", "Écoute libre")];
const CONTEXT_MUTE: &[(&str, &str)] = &[("auto", "Automatique (jeux, visios)"), ("on", "Toujours en sourdine"), ("off", "Jamais")];
const TONE_LABELS: &[(&str, &str)] = &[
    ("complice", "Complice et chaleureux"),
    ("majordome", "Majordome britannique"),
    ("professionnel", "Professionnel"),
    ("direct", "Direct et factuel"),
    ("enjoue", "Enjoué"),
];
const VERBOSITY_LABELS: &[(&str, &str)] = &[("tres-court", "Une phrase"), ("court", "Une à trois phrases"), ("detaille", "Détaillé (5 phrases max)")];
const SPEAKER_TYPES: &[&str] = &["moi", "femme", "mari", "fille", "fils", "famille", "ami", "invite", "autre"];

fn map_of(pairs: &[(&str, &str)]) -> J {
    J::Object(pairs.iter().map(|(k, v)| (k.to_string(), json!(v))).collect())
}

fn has_key(pairs: &[(&str, &str)], k: &str) -> bool {
    pairs.iter().any(|(x, _)| *x == k)
}

// --- Journal : libellés et résumés lisibles ----------------------------------------------------

pub const JOURNAL_LABELS: &[(&str, &str)] = &[
    ("demarrage", "Démarrage"), ("optimisation_auto", "Optimisation automatique"), ("compilation_nettoyee", "Compilations nettoyées"), ("spotify_configuration", "Spotify configuré"),
    ("demande", "Demande"), ("parole", "Jarvis a dit"), ("tache_debut", "Tâche lancée"),
    ("tache_fin", "Tâche terminée"), ("tache_annulee", "Tâche annulée"), ("outil", "Outil utilisé"),
    ("intention_locale", "Action locale"), ("clarification_demandee", "Question de Jarvis"),
    ("clarification_repondue", "Précision donnée"), ("clarification_expiree", "Question sans réponse"),
    ("validation_demandee", "Validation demandée"), ("validation_accordee", "Validation accordée"),
    ("validation_refusee", "Validation refusée"), ("validation_expiree", "Validation expirée"),
    ("validation_vocale_refusee", "Code dicté refusé"), ("acces_refuse", "Accès refusé"),
    ("memoire_ajout", "Mémoire : ajout"), ("annonce_reportee", "Annonce reportée"), ("presence_arrivee", "Présence : arrivée"), ("presence_depart", "Présence : départ"), ("presence_vie_privee", "Présence : vie privée"), ("memoire_oubli", "Mémoire : oubli"), ("memoire_modification", "Mémoire : modification"),
    ("memoire_reinitialisation", "Mémoire : purge"), ("action_systeme", "Action système"), ("action_systeme_erreur", "Erreur système"),
    ("action_google", "Action Google"), ("action_google_erreur", "Erreur Google"), ("auto_diagnostic", "Auto-diagnostic"),
    ("reglages_modifies", "Réglages modifiés"), ("administration_ouverte", "Administration ouverte"),
    ("zone_ajoutee", "Zone ajoutée"), ("zone_supprimee", "Zone retirée"), ("autorisation_temporaire", "Autorisation temporaire"),
    ("vocabulaire_ajout", "Vocabulaire : ajout"), ("vocabulaire_retrait", "Vocabulaire : retrait"), ("vocabulaire_correction", "Vocabulaire : correction"),
    ("voix_repli", "Voix : repli"), ("voix_livekit", "Voix : LiveKit"), ("voix_redemarrage", "Voix : redémarrage"),
    ("onboarding_termine", "Première rencontre"), ("securite_modifiee", "Sécurité modifiée"),
    ("gmail_configuration", "Gmail intégré"),
    ("google_docs_configuration", "Google Docs"), ("google_docs_connexion", "Google Docs : connexion"),
    ("reunion_detectee", "Réunion Meet détectée"), ("reunion_debut", "Réunion : début"), ("reunion_fin", "Réunion : fin"),
    ("reunion_archivee", "Réunion archivée"), ("reunion_voix_nommee", "Réunion : voix nommée"), ("reunion_resume", "Réunion : résumé"),
    ("reunion_google_doc", "Réunion : Google Doc"), ("reunions_recherche", "Réunions : recherche"), ("reunion_lue", "Réunion lue par l’IA"),
    ("mcp_ajoute", "Connecteur ajouté"), ("mcp_modifie", "Connecteur modifié"), ("mcp_supprime", "Connecteur retiré"), ("mcp_teste", "Connecteur testé"),
    ("enrolement_locuteur_demarre", "Empreinte vocale : apprentissage"),
    ("enrolement_locuteur_termine", "Empreinte vocale : enregistrée"),
    ("changement_locuteur", "Empreinte vocale : réglages"),
    ("changement_apparence_coeur", "Apparence du cœur"),
    ("reinitialisation_locuteur", "Empreinte vocale : réinitialisation"),
    ("suppression_locuteur", "Empreinte vocale : suppression"),
    ("renommage_locuteur", "Empreinte vocale : renommage"),
    ("changement_locuteur_principal", "Empreinte vocale : profil principal"),
    ("changement_mode_locuteur", "Empreinte vocale : mode filtrage"),
    ("changement_seuil_locuteur", "Empreinte vocale : sensibilité"),
    ("echec_statut", "Échec : statut"), ("echec_apprentissage", "Échec confié à Claude Code"),
    ("apprentissage_demande", "Demande confiée à Claude Code"), ("commentaire_action", "Commentaire → Claude Code"),
    ("configuration_demandee", "Configuration à compléter"), ("arret_general", "Arrêt général (STOP)"), ("telegram_appaire", "Telegram relié"),
    ("telegram_refuse", "Telegram : conversation étrangère refusée"), ("notification", "Notification"), ("demande_ignoree", "Demande ignorée"),
    ("action_locale", "Action locale (sans IA)"),
    ("action_locale_erreur", "Erreur action locale"),
    ("relance_jarvis", "Relance de Jarvis"), ("relance_demandee", "Relance demandée"), ("relance_recompilation", "Recompilation"),
    ("tmux_relais_debut", "tmux : relais ouvert"), ("tmux_relais_fin", "tmux : relais fermé"),
    ("terminal_action", "Terminal : action lancée"), ("terminal_validation_douteuse", "Terminal : Entrée sur voix douteuse"),
    ("terminal_validation_differee", "Terminal : Entrée après sortie du terminal"), ("terminal_actions_modifiees", "Terminal : actions modifiées"),
    ("embranchement_constate", "Journal : embranchement constaté"), ("instance_refusee", "Second lancement refusé"),
    ("apprentissage_locuteur", "Empreinte vocale : phrase apprise"), ("nouveau_locuteur", "Empreinte vocale : nouvelle personne"),
    ("voix_non_reconnue", "Voix non reconnue"),
    ("maison_hue_associe", "Maison : pont Hue associé"), ("maison_hue_oublie", "Maison : pont Hue oublié"), ("maison_prise_ajoutee", "Maison : prise ajoutée"), ("maison_prise_retiree", "Maison : prise retirée"),
    ("colis_statut", "Colis : statut mis à jour"),
    ("documents_dossier_ajoute", "Documents : dossier ajouté"), ("documents_dossier_retire", "Documents : dossier retiré"), ("documents_recherche", "Documents : recherche de l'IA"), ("savoir_recherche", "Data : recherche lancée"), ("savoir_fin", "Data : dossier mis à jour"), ("savoir_supprime", "Data : dossier supprimé"), ("savoir_question", "Data : question de l'IA"),
];

pub fn journal_label(t: &str) -> String {
    JOURNAL_LABELS.iter().find(|(k, _)| *k == t).map(|(_, v)| v.to_string()).unwrap_or_else(|| t.to_string())
}

/// Types qui composent la conversation (vue « Dialogues »).
pub const DIALOGUE_TYPES: &[&str] = &[
    "demande", "parole", "tache_fin", "intention_locale", "clarification_demandee", "validation_demandee", "validation_accordee", "validation_refusee", "tache_annulee",
];

fn short_value(v: &J, max: usize) -> String {
    let t = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if t.chars().count() > max {
        format!("{}…", js::slice_chars(&t, max - 1))
    } else {
        t
    }
}

fn join_present(parts: &[&J]) -> String {
    parts.iter().filter(|v| js::truthy(v)).map(|v| js::s(v)).collect::<Vec<_>>().join(" · ")
}

pub fn summarize(r: &J) -> String {
    let d = &r["data"];
    let g = |k: &str| js::gs(d, k);
    match js::gs(r, "type").as_str() {
        "demande" => {
            if js::truthy(&d["locuteur"]) {
                format!("[{}] {}", g("locuteur"), g("texte"))
            } else {
                g("texte")
            }
        }
        "parole" => g("texte"),
        "tache_debut" => format!(
            "{}{}{}{} — {}",
            g("moteur"),
            if js::truthy(&d["modele"]) { format!(" · {}", g("modele")) } else { String::new() },
            if js::truthy(&d["fond"]) { " · fond" } else { "" },
            if js::truthy(&d["locuteur"]) { format!(" · [{}]", g("locuteur")) } else { String::new() },
            g("demande")
        ),
        "tache_fin" => {
            if g("statut") == "terminee" {
                Some(g("reponse")).filter(|s| !s.is_empty()).unwrap_or_else(|| "(sans réponse)".into())
            } else {
                format!("échec{}", if js::truthy(&d["erreur"]) { format!(" : {}", g("erreur")) } else { String::new() })
            }
        }
        "outil" => format!("{} — {}", g("outil"), g("detail")),
        "intention_locale" => format!("{} → {}", g("texte"), g("reponse")),
        "clarification_demandee" => g("question"),
        "clarification_repondue" => format!("{} → {}", g("question"), g("reponse")),
        "validation_demandee" | "validation_accordee" | "validation_refusee" | "validation_expiree" => {
            let first = if js::truthy(&d["outil"]) { &d["outil"] } else { &d["action"] };
            join_present(&[first, &d["detail"], &d["niveau"], &d["type"], &d["origine"]])
        }
        "acces_refuse" => join_present(&[&d["outil"], &d["detail"], &d["motif"]]),
        "memoire_ajout" | "memoire_oubli" => format!("[{}] {}", g("categorie"), g("texte")),
        "annonce_reportee" => format!("{} ({})", g("source"), g("motif")),
        "memoire_modification" => format!("{} → {}", g("avant"), g("apres")),
        "enrolement_locuteur_demarre" => format!(
            "Apprentissage : {} ({} échantillons)",
            Some(g("nom")).filter(|s| !s.is_empty()).unwrap_or_else(|| "Utilisateur".into()),
            if js::truthy(&d["echantillons"]) { g("echantillons") } else { "3".into() }
        ),
        "enrolement_locuteur_termine" => {
            let name = if js::truthy(&d["speaker"]) {
                js::gs(&d["speaker"], "name")
            } else {
                Some(js::gs(&d["status"], "targetName")).filter(|s| !s.is_empty()).unwrap_or_else(|| "OK".into())
            };
            format!("Profil enregistré : {name}")
        }
        "suppression_locuteur" => format!("Suppression : {}", if js::truthy(&d["nom"]) { g("nom") } else { g("id") }),
        "renommage_locuteur" => format!("Renommage : {} → {}", g("id"), g("nom")),
        "changement_locuteur_principal" => format!("Principal : {}", g("id")),
        "embranchement_constate" => format!(
            "{} bifurcation(s) du {} au {} ({} entrées) — {}",
            js::gn(d, "bifurcations").unwrap_or(0.0) as i64,
            Some(g("debut")).filter(|s| !s.is_empty()).unwrap_or_else(|| "?".into()),
            Some(g("fin")).filter(|s| !s.is_empty()).unwrap_or_else(|| "?".into()),
            d["entrees"].as_array().map(|a| a.len()).unwrap_or(0),
            g("motif")
        ),
        "apprentissage_locuteur" => format!(
            "{} : phrase apprise ({} au total){}",
            if js::truthy(&d["nom"]) { g("nom") } else { g("id") },
            js::gn(d, "appris").unwrap_or(0.0) as i64,
            if js::truthy(&d["texte"]) { format!(" — « {} »", g("texte")) } else { String::new() }
        ),
        "nouveau_locuteur" => format!(
            "{} ({}){}",
            g("nom"),
            Some(g("lien")).filter(|s| !s.is_empty()).unwrap_or_else(|| "autre".into()),
            if js::truthy(&d["texte"]) { format!(" — « {} »", g("texte")) } else { String::new() }
        ),
        "voix_non_reconnue" => format!(
            "{} {}{} % — « {} »",
            g("verdict"),
            if js::truthy(&d["proche"]) { format!("({} ?) ", g("proche")) } else { String::new() },
            (js::gn(d, "similarite").unwrap_or(0.0) * 100.0).round() as i64,
            g("texte")
        ),
        _ => short_value(d, 160),
    }
}

/// Méthodes que la fenêtre d'administration a le droit d'appeler (liste blanche).
pub const ADMIN_METHODS: &[&str] = &[
    "snapshot", "catalog", "setStatus", "previewVoice", "setConfig",
    "memoryAdd", "memoryUpdate", "memoryDelete", "memoryPurgeRequest", "memoryPurgeConfirm", "memoryAutoAnalyze",
    "emotionAdd", "emotionUpdate", "emotionDelete", "emotionReset",
    "zoneAdd", "zoneAddConfirm", "zoneRemove", "vocabAdd", "vocabRemove", "vocabCorrect",
    "voiceRestart", "journalQuery", "journalStats", "journalVerify",
    "securityList", "securitySet", "securitySetConfirm", "pendingDeny",
    "mcpState", "mcpAdd", "mcpUpdate", "mcpToggle", "mcpRemove", "mcpTest", "mcpAuth",
    "gmailState", "gmailConfigure", "gmailTest", "gmailForget", "openHelpLink", "openHeartPreview", "companionState", "companionSet", "companionRestart",
    "gdocsState", "gdocsConfigure", "gdocsConnect", "gdocsForget",
    "projectsState", "projectsSetRoot", "projectsRefresh", "projectOpen", "projectClaude",
    "pronunciationState", "pronunciationSet", "pronunciationRemove", "pronunciationTest", "pronunciationCompare",
    "terminalActionsState", "terminalActionsSave", "terminalActionsReset",
    "telegramState", "telegramConfigure", "telegramSet", "telegramUnpair", "telegramTest", "telegramForget",
    "nomadeState", "nomadePair", "nomadeAccountRequest", "nomadeAccountConfirm", "nomadeShare", "nomadeDropRequest", "nomadeDropConfirm",
    "nomadeOptions", "nomadeSync", "nomadeSessions", "nomadeRevoke", "nomadeDepot", "nomadeConflict", "nomadeForget",
    "nomadeGuests", "nomadeGuestRequest", "nomadeGuestConfirm", "nomadeGuestInvite", "nomadeGuestDelete", "nomadeCheckSite",
    "spotifyState", "spotifyConfigure", "spotifyTest", "spotifyTestLocal", "spotifyActivateDirectMode", "spotifyForget",
    "speakerState", "speakerSet", "speakerReset", "speakerEnroll", "speakerCancel",
    "speakerDelete", "speakerRename", "speakerPrimary", "speakerSetType", "speakerOptions",
    "macroDelete",
    "transferState", "transferExport", "transferPreview", "transferImportRequest", "transferImportConfirm", "transferOpenFolder",
    "corpusState", "corpusRecord", "corpusClear", "benchRun", "speakerApplyThresholds",
    "voiceSetupState", "audioSwitch", "perfMetrics", "voiceQuality",
    "documentsState", "documentsAdd", "documentsRemove", "documentsScan",
    "homeState", "homeHuePair", "homeHueRefresh", "homeHueForget", "homePlugAdd", "homePlugRemove", "homeTest",
    "failuresList", "failureSetStatus", "failureDelete", "failureLearn", "journalLearn", "journalComment",
];

struct Challenge {
    code: String,
    action: String,
    payload: J,
    label: String,
    expires_at: i64,
}

pub struct AdminService {
    orch: Rc<Orchestrator>,
    gateway_port: u16,
    get_metrics: Box<dyn Fn() -> J>,
    voice: Rc<VoiceSupervisor>,
    on_config_change: Rc<dyn Fn(&[String], &J)>,
    challenges: RefCell<HashMap<String, Challenge>>,
    bench_state: Rc<RefCell<Option<J>>>,
}

fn arg(a: &[J], i: usize) -> J {
    a.get(i).cloned().unwrap_or(Value::Null)
}

fn sarg(a: &[J], i: usize) -> String {
    js::s(&arg(a, i))
}

fn mask_email(e: &str) -> String {
    crate::re!(r"^(.{2})(.*)(@.*)$").replace(e, "$1***$3").to_string()
}

fn spread(base: J, extra: J) -> J {
    let mut b = base;
    js::assign(&mut b, extra);
    b
}

impl AdminService {
    pub fn new(orch: Rc<Orchestrator>, gateway_port: u16, voice: Rc<VoiceSupervisor>, get_metrics: Box<dyn Fn() -> J>, on_config_change: Rc<dyn Fn(&[String], &J)>) -> Rc<Self> {
        Rc::new(Self {
            orch,
            gateway_port,
            get_metrics,
            voice,
            on_config_change,
            challenges: RefCell::new(HashMap::new()),
            bench_state: Rc::new(RefCell::new(None)),
        })
    }

    /// Appel d'une méthode de la liste blanche (arguments tels qu'envoyés par la page).
    pub fn call(self: &Rc<Self>, method: &str, a: Vec<J>) -> Fut<Result<J, String>> {
        let me = self.clone();
        let method = method.to_string();
        Box::pin(async move {
            if !ADMIN_METHODS.contains(&method.as_str()) {
                return Err(format!("méthode inconnue : {method}"));
            }
            let a = a.as_slice();
            Ok(match method.as_str() {
                "snapshot" => me.snapshot(),
                "catalog" => me.catalog(),
                "setStatus" => me.set_status(&arg(a, 0)).await,
                "previewVoice" => me.preview_voice(),
                "setConfig" => me.set_config(&arg(a, 0)),
                "memoryAdd" => me.memory_add(&sarg(a, 0), &sarg(a, 1), &sarg(a, 2)),
                "memoryUpdate" => me.memory_update(&sarg(a, 0), &sarg(a, 1), &sarg(a, 2), (a.len() > 3).then(|| sarg(a, 3))),
                "memoryDelete" => me.memory_delete(&sarg(a, 0)),
                "memoryAutoAnalyze" => {
                    let r = me.orch.memory_extract.analyze().await;
                    json!({ "ok": js::gb(&r, "ok"), "error": r["error"], "entry": r["entry"], "statut": r["statut"], "snapshot": me.snapshot() })
                }
                "memoryPurgeRequest" => me.challenge("memory_purge", json!({}), "Effacer toute la mémoire (RAG)"),
                "memoryPurgeConfirm" => me.memory_purge_confirm(&sarg(a, 0), &sarg(a, 1)),
                "emotionAdd" => spread(me.orch.emotions.add(&sarg(a, 0), &arg(a, 1)), json!({ "snapshot": me.snapshot() })),
                "emotionUpdate" => {
                    let text = arg(a, 1);
                    let score = arg(a, 2);
                    let t = if text.is_null() { None } else { Some(js::s(&text)) };
                    let r = me.orch.emotions.update(&sarg(a, 0), t.as_deref(), if score.is_null() { None } else { Some(&score) });
                    spread(r, json!({ "snapshot": me.snapshot() }))
                }
                "emotionDelete" => spread(me.orch.emotions.delete(&sarg(a, 0)), json!({ "snapshot": me.snapshot() })),
                "emotionReset" => {
                    me.orch.emotions.reset_defaults();
                    json!({ "ok": true, "snapshot": me.snapshot() })
                }
                "zoneAdd" => me.zone_add(&sarg(a, 0), &sarg(a, 1), &sarg(a, 2), &sarg(a, 3)),
                "zoneAddConfirm" => match me.consume(&sarg(a, 0), &sarg(a, 1), "zone_add") {
                    Ok(p) => me.apply_zone_add(&p),
                    Err(e) => e,
                },
                "zoneRemove" => me.zone_remove(&sarg(a, 0)),
                "vocabAdd" => me.vocab_add(&sarg(a, 0)),
                "vocabRemove" => me.vocab_remove(&sarg(a, 0)),
                "vocabCorrect" => me.vocab_correct(&sarg(a, 0), &sarg(a, 1)),
                "voiceRestart" => {
                    me.voice.restart();
                    me.orch.record("voix_redemarrage", json!({ "origine": "administration" }));
                    json!({ "ok": true })
                }
                "journalQuery" => me.journal_query(&arg(a, 0)),
                "journalStats" => me.journal_stats(),
                "journalVerify" => {
                    let res = me.orch.history.verify();
                    me.orch.publish_journal();
                    json!({ "ok": true, "integrity": res })
                }
                "securityList" => me.security_list(),
                "securitySet" => me.security_set(&sarg(a, 0), &sarg(a, 1)),
                "securitySetConfirm" => match me.consume(&sarg(a, 0), &sarg(a, 1), "security_lower") {
                    Ok(p) => me.apply_security(&js::gs(&p, "key"), &js::gs(&p, "level")),
                    Err(e) => e,
                },
                "pendingDeny" => {
                    let id = sarg(a, 0);
                    if !me.orch.pending.borrow().iter().any(|(k, _)| *k == id) {
                        json!({ "ok": false, "error": "validation introuvable" })
                    } else {
                        me.orch.deny(&id);
                        json!({ "ok": true, "list": me.security_list() })
                    }
                }
                "mcpState" => me.mcp_state(),
                "mcpAdd" => me.mcp_add(&arg(a, 0)),
                "mcpUpdate" => me.mcp_update(&sarg(a, 0), &arg(a, 1)),
                "mcpToggle" => me.mcp_update(&sarg(a, 0), &json!({ "enabled": js::truthy(&arg(a, 1)) })),
                "mcpRemove" => me.mcp_remove(&sarg(a, 0)),
                "mcpTest" => me.mcp_test(&sarg(a, 0)).await,
                "mcpAuth" => me.mcp_auth(&sarg(a, 0)).await,
                "gmailState" => me.gmail_state(),
                "gmailConfigure" => me.gmail_configure(&sarg(a, 0), &sarg(a, 1)).await,
                "gmailTest" => me.gmail_test().await,
                "gmailForget" => {
                    me.orch.google.save_config(json!({ "email": "", "appPassword": "" }));
                    me.orch.record("gmail_configuration", json!({ "retrait": true, "origine": "administration" }));
                    json!({ "ok": true, "gmail": me.gmail_state() })
                }
                "openHelpLink" => me.open_help_link(&sarg(a, 0)).await,
                "openHeartPreview" => me.open_heart_preview(&arg(a, 0)).await,
                "companionState" => me.orch.companion.view(),
                "companionSet" => me.orch.companion.set_config(&arg(a, 0)),
                "companionRestart" => me.orch.companion.restart(),
                "gdocsState" => me.orch.google_docs.state(),
                "gdocsConfigure" => {
                    let r = me.orch.google_docs.set_client(&sarg(a, 0), &sarg(a, 1));
                    me.orch.record("google_docs_configuration", json!({ "ok": r["ok"], "erreur": r["error"], "origine": "administration" }));
                    spread(r, json!({ "gdocs": me.orch.google_docs.state() }))
                }
                "gdocsConnect" => {
                    let r = me.orch.google_docs.connect().await;
                    me.orch.record("google_docs_connexion", json!({ "ok": r["ok"], "erreur": r["error"], "origine": "administration" }));
                    spread(r, json!({ "gdocs": me.orch.google_docs.state() }))
                }
                "gdocsForget" => {
                    me.orch.google_docs.forget();
                    me.orch.record("google_docs_configuration", json!({ "retrait": true, "origine": "administration" }));
                    json!({ "ok": true, "gdocs": me.orch.google_docs.state() })
                }
                "projectsState" => me.projects_state().await,
                "projectsSetRoot" => me.projects_set_root(&sarg(a, 0)).await,
                "projectsRefresh" => {
                    me.orch.projects.invalidate();
                    me.projects_state().await
                }
                "projectOpen" => me.project_open(&sarg(a, 0)).await,
                "projectClaude" => me.project_claude(&sarg(a, 0)).await,
                "pronunciationState" => me.pronunciation_state(),
                "pronunciationSet" => {
                    let r = me.orch.pronunciation.set_for(&sarg(a, 0), &sarg(a, 1), &sarg(a, 2));
                    if !js::gb(&r, "ok") {
                        r
                    } else {
                        me.orch.record("reglages_modifies", json!({ "prononciation": format!("{} → {}", js::gs(&r, "word"), js::gs(&r, "say")), "origine": "administration" }));
                        me.pronunciation_state()
                    }
                }
                "pronunciationRemove" => {
                    let voice = arg(a, 1);
                    me.orch.pronunciation.remove_for(&sarg(a, 0), voice.as_str());
                    me.pronunciation_state()
                }
                "pronunciationTest" => {
                    let t = js::slice_chars(sarg(a, 0).trim(), 300);
                    if t.is_empty() {
                        json!({ "ok": false, "error": "texte vide" })
                    } else {
                        me.orch.emit(json!({ "type": "say", "text": t, "diction": true }));
                        let voice = me.orch.status.d.borrow().voice.clone();
                        json!({ "ok": true, "spoken": me.orch.pronunciation.apply_for(&t, &voice) })
                    }
                }
                // Même phrase dite par une autre voix (comparer avec une voix multilingue), sans la choisir.
                "pronunciationCompare" => {
                    let (t, voice) = (js::slice_chars(sarg(a, 0).trim(), 300), sarg(a, 1));
                    if t.is_empty() || !super::status::VOICES.iter().any(|v| v.1 == voice) {
                        json!({ "ok": false, "error": "texte ou voix invalide" })
                    } else {
                        me.orch.emit(json!({ "type": "say", "text": me.orch.pronunciation.apply_for(&t, &voice), "voice": voice, "diction": true }));
                        json!({ "ok": true })
                    }
                }
                "terminalActionsState" => json!({ "ok": true, "actions": me.orch.terminal_actions.list() }),
                "terminalActionsSave" => {
                    let r = me.orch.terminal_actions.save(&arg(a, 0));
                    if js::gb(&r, "ok") {
                        me.orch.record("terminal_actions_modifiees", json!({ "actions": js::arr(&r["actions"]).len(), "origine": "administration" }));
                        me.orch.terminal_voice.refresh_all_cards();
                        r
                    } else {
                        json!({ "ok": false, "error": js::str_list(&r["errors"]).join(" ") })
                    }
                }
                "terminalActionsReset" => {
                    let r = me.orch.terminal_actions.reset();
                    me.orch.record("terminal_actions_modifiees", json!({ "actions": js::arr(&r["actions"]).len(), "origine": "administration", "defaut": true }));
                    me.orch.terminal_voice.refresh_all_cards();
                    r
                }
                "telegramState" => spread(json!({ "ok": true }), me.orch.telegram.state()),
                "telegramConfigure" => {
                    let r = me.orch.telegram.configure(&sarg(a, 0)).await;
                    if js::gb(&r, "ok") {
                        me.orch.record("reglages_modifies", json!({ "telegram": format!("@{}", js::gs(&r, "bot")), "origine": "administration" }));
                    }
                    r
                }
                "telegramSet" => me.orch.telegram.set(&if arg(a, 0).is_object() { arg(a, 0) } else { json!({}) }),
                "telegramUnpair" => {
                    me.orch.record("reglages_modifies", json!({ "telegram": "désappairé", "origine": "administration" }));
                    me.orch.telegram.unpair()
                }
                "telegramTest" => {
                    let tg = &me.orch.telegram;
                    if !js::gb(&tg.state(), "paired") {
                        json!({ "ok": false, "error": "aucune conversation appairée" })
                    } else if tg.send_now("👋 Message de test de Jarvis.").await {
                        json!({ "ok": true })
                    } else {
                        let e = tg.last_error.borrow().clone();
                        json!({ "ok": false, "error": if e.is_empty() { "envoi impossible".to_string() } else { e } })
                    }
                }
                "telegramForget" => {
                    me.orch.record("reglages_modifies", json!({ "telegram": "oublié", "origine": "administration" }));
                    me.orch.telegram.forget()
                }
                "nomadeState" => spread(json!({ "ok": true }), me.orch.nomade.state()),
                "nomadePair" => {
                    let r = me.orch.nomade.pair(&sarg(a, 0), &sarg(a, 1), &sarg(a, 2)).await;
                    if js::gb(&r, "ok") {
                        me.orch.record("reglages_modifies", json!({ "nomade": "relié", "site": js::gs(&r, "url"), "origine": "administration" }));
                    }
                    r
                }
                // Changer l'accès au site : code de validation, comme les autres réglages sensibles.
                "nomadeAccountRequest" => {
                    let (login, password) = (sarg(a, 0).trim().to_string(), sarg(a, 1));
                    if !super::nomade::valid_login(&login) || (!password.is_empty() && password.chars().count() < 10) {
                        json!({ "ok": false, "error": "Identifiant de 3 caractères au moins (lettres, chiffres, . _ @ -) et mot de passe de 10 caractères au moins." })
                    } else {
                        spread(
                            me.challenge("nomade_account", json!({ "login": login, "password": password, "revoke": js::truthy(&arg(a, 2)) }), "Changer l'identifiant ou le mot de passe du site Nomade"),
                            json!({ "confirm": "nomadeAccountConfirm" }),
                        )
                    }
                }
                "nomadeAccountConfirm" => match me.consume(&sarg(a, 0), &sarg(a, 1), "nomade_account") {
                    Err(e) => e,
                    Ok(p) => {
                        let r = me.orch.nomade.change_account(&js::gs(&p, "login"), &js::gs(&p, "password"), js::gb(&p, "revoke")).await;
                        if js::gb(&r, "ok") {
                            me.orch.record("reglages_modifies", json!({ "nomade": "accès au site modifié", "identifiant": js::gs(&p, "login"), "origine": "administration" }));
                        }
                        r
                    }
                },
                "nomadeShare" => {
                    let r = me.orch.nomade.share(&sarg(a, 0));
                    if js::gb(&r, "ok") {
                        me.orch.record("reglages_modifies", json!({ "nomade": "partage", "source": sarg(a, 0), "origine": "administration" }));
                    }
                    r
                }
                // Retirer une source efface ses données en ligne : code de validation.
                "nomadeDropRequest" => {
                    let id = sarg(a, 0);
                    let label = js::arr(&me.orch.nomade.state()["sources"]).into_iter().find(|s| js::gs(s, "id") == id).map(|s| js::gs(&s, "label")).unwrap_or_else(|| id.clone());
                    spread(me.challenge("nomade_drop", json!({ "id": id }), &format!("Ne plus partager « {label} » et l'effacer du site Nomade")), json!({ "confirm": "nomadeDropConfirm" }))
                }
                "nomadeDropConfirm" => match me.consume(&sarg(a, 0), &sarg(a, 1), "nomade_drop") {
                    Err(e) => e,
                    Ok(p) => {
                        let r = me.orch.nomade.drop_source(&js::gs(&p, "id")).await;
                        if js::gb(&r, "ok") {
                            me.orch.record("reglages_modifies", json!({ "nomade": "retrait", "source": js::gs(&p, "id"), "origine": "administration" }));
                        }
                        r
                    }
                },
                "nomadeOptions" => me.orch.nomade.set_options(&if arg(a, 0).is_object() { arg(a, 0) } else { json!({}) }),
                "nomadeSync" => me.orch.nomade.sync_now().await,
                "nomadeSessions" => me.orch.nomade.sessions().await,
                "nomadeRevoke" => {
                    me.orch.record("reglages_modifies", json!({ "nomade": "appareil révoqué", "origine": "administration" }));
                    me.orch.nomade.revoke(arg(a, 0).as_i64().unwrap_or(0)).await
                }
                "nomadeDepot" => me.orch.nomade.depot(arg(a, 0).as_i64().unwrap_or(0), &sarg(a, 1)).await,
                "nomadeConflict" => me.orch.nomade.conflict(&sarg(a, 0), arg(a, 1).as_i64().unwrap_or(0), &sarg(a, 2)),
                "nomadeGuests" => me.orch.nomade.guests().await,
                "nomadeCheckSite" => {
                    me.orch.nomade.check_site().await;
                    spread(json!({ "ok": true }), me.orch.nomade.state())
                }
                // Donner accès à des données à une autre personne : code de validation (création et modification).
                "nomadeGuestRequest" => {
                    let p = if arg(a, 0).is_object() { arg(a, 0) } else { json!({}) };
                    let login = js::gs(&p, "login").trim().to_string();
                    if !super::nomade::valid_login(&login) {
                        json!({ "ok": false, "error": "Identifiant de 3 caractères au moins (lettres, chiffres, . _ @ -)." })
                    } else {
                        let n = js::arr(&p["sources"]).len();
                        let mode = if js::gs(&p, "mode") == "ecriture" { "lecture et écriture" } else { "lecture seule" };
                        let who = if js::gs(&p, "email").is_empty() { login.clone() } else { format!("{login} ({})", js::gs(&p, "email")) };
                        let label = if p["id"].as_i64().unwrap_or(0) > 0 {
                            format!("Modifier le partage Nomade de {who} : {n} source(s), {mode}")
                        } else {
                            format!("Partager {n} source(s) Nomade avec {who}, {mode}, et lui envoyer l'invitation")
                        };
                        spread(me.challenge("nomade_guest", p, &label), json!({ "confirm": "nomadeGuestConfirm" }))
                    }
                }
                "nomadeGuestConfirm" => match me.consume(&sarg(a, 0), &sarg(a, 1), "nomade_guest") {
                    Err(e) => e,
                    Ok(p) => {
                        let r = me.orch.nomade.guest_save(&p).await;
                        if js::gb(&r, "ok") {
                            me.orch.record("reglages_modifies", json!({ "nomade": "partage", "identifiant": js::gs(&p, "login"), "mode": p["mode"], "sources": p["sources"], "origine": "administration" }));
                        }
                        r
                    }
                },
                "nomadeGuestInvite" => me.orch.nomade.guest_invite(arg(a, 0).as_i64().unwrap_or(0)).await,
                "nomadeGuestDelete" => {
                    let r = me.orch.nomade.guest_delete(arg(a, 0).as_i64().unwrap_or(0)).await;
                    if js::gb(&r, "ok") {
                        me.orch.record("reglages_modifies", json!({ "nomade": "partage supprimé", "id": arg(a, 0), "origine": "administration" }));
                    }
                    r
                }
                "nomadeForget" => {
                    me.orch.record("reglages_modifies", json!({ "nomade": "oublié", "origine": "administration" }));
                    me.orch.nomade.forget()
                }
                "spotifyState" => me.spotify_state(),
                "spotifyConfigure" => me.spotify_configure(&sarg(a, 0), &sarg(a, 1), &arg(a, 2)).await,
                "spotifyTest" => {
                    let t = me.orch.spotify.test().await;
                    spread(t, json!({ "spotify": me.spotify_state() }))
                }
                "spotifyTestLocal" => me.spotify_test_local(),
                "spotifyActivateDirectMode" => {
                    let r = me.orch.spotify.activate_direct_mode();
                    me.orch.record("spotify_configuration", json!({ "ok": true, "mode": "direct", "origine": "administration" }));
                    spread(r, json!({ "spotify": me.spotify_state() }))
                }
                "spotifyForget" => {
                    me.orch.spotify.forget();
                    me.orch.record("spotify_configuration", json!({ "retrait": true, "origine": "administration" }));
                    json!({ "ok": true, "spotify": me.spotify_state() })
                }
                "speakerState" => json!({ "ok": true, "speaker": me.orch.get_speaker_status() }),
                "speakerSet" => me.speaker_set(&arg(a, 0)),
                "speakerReset" => me.speaker_reset(),
                "speakerEnroll" => me.speaker_enroll(&arg(a, 0), &arg(a, 1)),
                "speakerCancel" => {
                    me.orch.emit(json!({ "type": "speaker_enroll_cancel" }));
                    json!({ "ok": true, "message": "Enrôlement annulé" })
                }
                "speakerDelete" => me.speaker_delete(&arg(a, 0)),
                "speakerRename" => me.speaker_rename(&arg(a, 0), &arg(a, 1)),
                "speakerPrimary" => me.speaker_primary(&arg(a, 0)),
                "speakerSetType" => me.speaker_set_type(&arg(a, 0), &sarg(a, 1)),
                "speakerOptions" => me.speaker_options(&arg(a, 0)),
                "macroDelete" => {
                    let name = sarg(a, 0);
                    if me.orch.macros.remove(&name) == 0 {
                        json!({ "ok": false, "error": "raccourci introuvable" })
                    } else {
                        me.orch.record("macro_supprimee", json!({ "nom": name, "origine": "administration" }));
                        json!({ "ok": true, "snapshot": me.snapshot() })
                    }
                }
                "corpusState" => {
                    let st = super::voice_corpus::corpus_state(&me.data_dir());
                    spread(spread(json!({ "ok": true }), st), json!({ "bench": *me.bench_state.borrow() }))
                }
                "corpusRecord" => me.corpus_record(&arg(a, 0)),
                "corpusClear" => {
                    let removed = super::voice_corpus::clear_corpus(&me.data_dir());
                    me.orch.record("corpus_vocal_supprime", json!({ "fichiers": removed, "origine": "administration" }));
                    json!({ "ok": true, "removed": removed })
                }
                "benchRun" => me.bench_run(&arg(a, 0)),
                "speakerApplyThresholds" => me.speaker_apply_thresholds(&arg(a, 0)),
                "voiceSetupState" => me.voice_setup_state().await,
                "audioSwitch" => me.audio_switch(&sarg(a, 0)).await,
                "perfMetrics" => me.perf_metrics(),
                "transferState" => me.transfer_state(),
                "transferExport" => me.transfer_export(&sarg(a, 0)).await,
                "transferPreview" => match me.transfer_file(&sarg(a, 0)) {
                    Err(e) => e,
                    Ok(p) => match Self::transfer_open(p, sarg(a, 1)).await {
                        Ok(preview) => json!({ "ok": true, "apercu": preview }),
                        Err(e) => json!({ "ok": false, "error": e }),
                    },
                },
                // Remplacer toutes les données : archive vérifiée d'abord, puis code de validation.
                "transferImportRequest" => match me.transfer_file(&sarg(a, 0)) {
                    Err(e) => e,
                    Ok(p) => match Self::transfer_open(p, sarg(a, 1)).await {
                        Err(e) => json!({ "ok": false, "error": e }),
                        Ok(preview) => spread(
                            me.challenge("transfer_import", json!({ "nom": sarg(a, 0), "motDePasse": sarg(a, 1) }), &format!("Remplacer toutes tes données par l'archive « {} »", sarg(a, 0))),
                            json!({ "confirm": "transferImportConfirm", "apercu": preview }),
                        ),
                    },
                },
                "transferImportConfirm" => match me.consume(&sarg(a, 0), &sarg(a, 1), "transfer_import") {
                    Err(e) => e,
                    Ok(p) => me.transfer_import(&js::gs(&p, "nom"), js::gs(&p, "motDePasse")).await,
                },
                "transferOpenFolder" => {
                    let dir = if sarg(a, 0) == "donnees" { me.data_dir() } else { me.transfer_dir() };
                    let _ = std::fs::create_dir_all(&dir);
                    crate::proc::detached("explorer.exe", &[&dir.to_string_lossy()]);
                    json!({ "ok": true })
                }
                "buildSpaceState" => json!({ "ok": true, "state": me.orch.build_space_state() }),
                "buildSpaceScan" => me.orch.build_space_scan(false).await,
                "buildSpaceSweep" => me.orch.build_space_sweep().await,
                "voiceQuality" => me.orch.voice_quality(),
                "homeState" => me.orch.home.state(),
                "homeHuePair" => match me.orch.home.hue_pair(&sarg(a, 0)).await {
                    Ok(v) => {
                        me.orch.record("maison_hue_associe", json!({ "origine": "administration" }));
                        v
                    }
                    Err(e) => json!({ "ok": false, "error": e }),
                },
                "homeHueRefresh" => match me.orch.home.hue_refresh().await {
                    Ok(_) => me.orch.home.state(),
                    Err(e) => json!({ "ok": false, "error": e }),
                },
                "homeHueForget" => {
                    me.orch.record("maison_hue_oublie", json!({ "origine": "administration" }));
                    me.orch.home.hue_forget()
                }
                "homePlugAdd" => match me.orch.home.add_plug(&sarg(a, 0), &sarg(a, 1), &sarg(a, 2)) {
                    Ok(v) => {
                        me.orch.record("maison_prise_ajoutee", json!({ "type": sarg(a, 1), "origine": "administration" }));
                        v
                    }
                    Err(e) => json!({ "ok": false, "error": e }),
                },
                "homePlugRemove" => {
                    me.orch.record("maison_prise_retiree", json!({ "origine": "administration" }));
                    me.orch.home.remove_plug(&sarg(a, 0))
                }
                "homeTest" => {
                    let r = me.orch.home.command(&json!({ "action": "basculer", "nom": sarg(a, 0) })).await;
                    if js::gb(&r, "ok") {
                        json!({ "ok": true, "state": me.orch.home.state() })
                    } else {
                        json!({ "ok": false, "error": if js::truthy(&r["message"]) { r["message"].clone() } else { r["error"].clone() } })
                    }
                }
                "documentsState" => me.orch.documents.state(),
                "documentsAdd" => {
                    let r = me.orch.documents.add_folder(&sarg(a, 0), &me.orch.zones);
                    match r {
                        Ok(v) => {
                            me.orch.record("documents_dossier_ajoute", json!({ "origine": "administration" }));
                            me.orch.documents.scan_soon();
                            v
                        }
                        Err(e) => json!({ "ok": false, "error": e }),
                    }
                }
                "documentsRemove" => {
                    let v = me.orch.documents.remove_folder(&sarg(a, 0));
                    me.orch.record("documents_dossier_retire", json!({ "origine": "administration" }));
                    v
                }
                "documentsScan" => json!({ "ok": me.orch.documents.scan_soon(), "error": if me.orch.documents.stats()["analyse"] == json!(true) { "Une analyse est déjà en cours." } else { "" } }),
                "failuresList" => me.failures_list(),
                "failureSetStatus" => {
                    match me.orch.failures.update(&sarg(a, 0), json!({ "statut": arg(a, 1) })) {
                        None => json!({ "ok": false, "error": "échec introuvable" }),
                        Some(item) => {
                            me.orch.record("echec_statut", json!({ "echec": item["id"], "statut": item["statut"], "demande": item["demande"] }));
                            me.failures_list()
                        }
                    }
                }
                "failureDelete" => {
                    me.orch.failures.remove(&sarg(a, 0));
                    me.failures_list()
                }
                "failureLearn" => me.failure_learn(&sarg(a, 0)),
                "journalLearn" => me.journal_learn(js::num(&arg(a, 0)).unwrap_or(-1.0) as i64),
                "journalComment" => me.journal_comment(js::num(&arg(a, 0)).unwrap_or(-1.0) as i64, &sarg(a, 1)),
                _ => return Err(format!("méthode inconnue : {method}")),
            })
        })
    }

    fn data_dir(&self) -> PathBuf {
        self.orch.data_dir.clone()
    }

    // --- Transfert des données (archive .jarvis, crate::transfer) -------------------------------------------------

    /// Les archives vivent dans le dossier des sauvegardes (réglage backup.dir, sinon Documents\Jarvis-sauvegardes).
    fn transfer_dir(&self) -> PathBuf {
        crate::backup::destination(&self.orch.config.get("backup"))
    }

    fn transfer_state(&self) -> J {
        let dir = self.transfer_dir();
        let mut items: Vec<J> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == crate::transfer::EXTENSION))
            .map(|e| {
                let m = e.metadata().ok();
                let at = m.as_ref().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Local>::from(t).to_rfc3339());
                json!({ "nom": e.file_name().to_string_lossy(), "octets": m.map(|m| m.len()).unwrap_or(0), "modifie": at })
            })
            .collect();
        items.sort_by_key(|i| std::cmp::Reverse(js::gs(i, "modifie")));
        json!({
            "ok": true,
            "dossier": dir.to_string_lossy(),
            "donnees": self.data_dir().to_string_lossy(),
            "archives": items,
            "importEnAttente": crate::transfer::pending_dir(&crate::paths::target_data_dir()).is_dir(),
        })
    }

    /// Une archive du dossier des sauvegardes, désignée par son seul nom (aucun autre chemin accepté).
    fn transfer_file(&self, name: &str) -> Result<PathBuf, J> {
        let plain = !name.is_empty() && !name.contains(['/', '\\', ':']) && name.ends_with(&format!(".{}", crate::transfer::EXTENSION));
        let p = self.transfer_dir().join(name);
        if plain && p.is_file() {
            Ok(p)
        } else {
            Err(json!({ "ok": false, "error": "archive introuvable dans le dossier des sauvegardes" }))
        }
    }

    /// Vérification complète (empreintes, mot de passe) hors du fil principal : Argon2 et la décompression prennent
    /// une à deux secondes.
    async fn transfer_open(path: PathBuf, password: String) -> Result<J, String> {
        tokio::task::spawn_blocking(move || crate::transfer::open(&path, Some(&password)).map(|a| a.preview())).await.map_err(|e| e.to_string())?
    }

    async fn transfer_export(&self, password: &str) -> J {
        if !password.is_empty() && password.chars().count() < 8 {
            return json!({ "ok": false, "error": "Mot de passe de 8 caractères au moins (ou aucun : les secrets des connecteurs ne sont alors pas exportés)." });
        }
        let (data, out, pw) = (self.data_dir(), self.transfer_dir().join(crate::transfer::default_name()), password.to_string());
        let target = out.clone();
        let done = tokio::task::spawn_blocking(move || {
            let sb = crate::secret_box::SecretBox::new();
            crate::transfer::export(&data, &target, Some(&pw), &|s| sb.decrypt(s))
        })
        .await;
        match done {
            Ok(Ok(r)) => {
                self.orch.record("donnees_exportees", json!({ "fichiers": r.files, "octets": r.bytes, "secrets": r.secrets, "chiffre": r.sealed, "origine": "administration" }));
                json!({ "ok": true, "fichier": out.to_string_lossy(), "fichiers": r.files, "octets": r.bytes, "secrets": r.secrets, "chiffre": r.sealed, "etat": self.transfer_state() })
            }
            Ok(Err(e)) => json!({ "ok": false, "error": e }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        }
    }

    /// Prépare l'import (appliqué à la relance, Jarvis fermé : paths.rs › relocate_guarded).
    async fn transfer_import(&self, name: &str, password: String) -> J {
        let path = match self.transfer_file(name) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let pending = crate::transfer::pending_dir(&crate::paths::target_data_dir());
        let done = tokio::task::spawn_blocking(move || {
            let sb = crate::secret_box::SecretBox::new();
            crate::transfer::open(&path, Some(&password)).and_then(|a| crate::transfer::stage(&a, &pending, &|s| sb.encrypt(s)))
        })
        .await;
        match done {
            Ok(Ok(n)) => {
                self.orch.record("donnees_import_prepare", json!({ "archive": name, "fichiers": n, "origine": "administration" }));
                json!({ "ok": true, "fichiers": n, "relancer": true, "etat": self.transfer_state() })
            }
            Ok(Err(e)) => json!({ "ok": false, "error": e }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        }
    }

    // --- Échecs et apprentissage -------------------------------------------------------------------

    fn failures_list(&self) -> J {
        json!({ "ok": true, "items": self.orch.failures.list(), "counts": self.orch.failures.counts() })
    }

    /// Confie un échec à Claude Code (fenêtre cmd, voir open_claude_cmd).
    fn failure_learn(&self, id: &str) -> J {
        let Some(item) = self.orch.failures.get(id) else { return json!({ "ok": false, "error": "échec introuvable" }) };
        let root = crate::paths::root().display().to_string();
        let file = self.write_prompt(&format!("echec-{}.md", js::gs(&item, "id")), &super::failures::learning_prompt(&item, &root));
        let res = self.open_claude_cmd(&file, &root);
        if !js::gb(&res, "ok") {
            return res;
        }
        self.orch.failures.update(
            &js::gs(&item, "id"),
            json!({ "statut": "en_cours", "apprentissage": { "at": js::iso_now(), "consigne": file, "ou": res["where"] } }),
        );
        self.orch.record("echec_apprentissage", json!({ "echec": item["id"], "demande": item["demande"], "ou": res["where"] }));
        spread(json!({ "ok": true, "where": res["where"] }), self.failures_list())
    }

    fn trace_line(r: &J) -> String {
        let at = js::gs(r, "at");
        let hh = at.get(11..19).unwrap_or("").to_string();
        format!("{hh} {} — {}", journal_label(&js::gs(r, "type")), js::slice_chars(&summarize(r), 300))
    }

    /// Journal › demande non exécutée : Claude Code cherche une alternative pour ce type de demande.
    fn journal_learn(&self, seq: i64) -> J {
        let all = self.orch.history.read(false);
        let Some(idx) = all.iter().position(|r| r["seq"].as_i64() == Some(seq)) else { return json!({ "ok": false, "error": "demande introuvable" }) };
        let r = &all[idx];
        if js::gs(r, "type") != "demande" {
            return json!({ "ok": false, "error": "demande introuvable" });
        }
        let outcome = super::failures::request_outcome(&all, idx);
        let mut trace = vec![];
        for (i, x) in all.iter().enumerate().skip(idx) {
            if trace.len() >= 25 || (i > idx && js::gs(x, "type") == "demande") {
                break;
            }
            trace.push(Self::trace_line(x));
        }
        let root = crate::paths::root().display().to_string();
        let d = &r["data"];
        let file = self.write_prompt(
            &format!("demande-{seq}.md"),
            &super::failures::request_prompt(
                &json!({ "seq": seq, "at": r["at"], "demande": js::gs(d, "texte"), "source": d["source"], "reason": outcome["reason"], "trace": trace }),
                &root,
            ),
        );
        let res = self.open_claude_cmd(&file, &root);
        if !js::gb(&res, "ok") {
            return res;
        }
        self.orch.record("apprentissage_demande", json!({ "seq": seq, "demande": js::gs(d, "texte"), "consigne": file }));
        json!({ "ok": true, "where": res["where"] })
    }

    /// Journal › commentaire sur n'importe quelle entrée : envoyé à Claude Code avec la demande
    /// d'origine et la suite de l'action dans le journal.
    fn journal_comment(&self, seq: i64, comment: &str) -> J {
        let text = js::slice_chars(comment.trim(), 4000);
        if text.is_empty() {
            return json!({ "ok": false, "error": "commentaire vide" });
        }
        let all = self.orch.history.read(false);
        let Some(idx) = all.iter().position(|r| r["seq"].as_i64() == Some(seq)) else { return json!({ "ok": false, "error": "entrée introuvable" }) };
        let r = &all[idx];
        // Demande à laquelle se rattache l'entrée : elle-même, ou la dernière demande qui la précède.
        let mut start = idx;
        if js::gs(r, "type") != "demande" {
            let mut i = idx as i64 - 1;
            while i >= 0 && idx as i64 - i <= 60 {
                if js::gs(&all[i as usize], "type") == "demande" {
                    start = i as usize;
                    break;
                }
                i -= 1;
            }
        }
        let req = (js::gs(&all[start], "type") == "demande").then(|| all[start].clone());
        let mut trace = vec![];
        for (i, x) in all.iter().enumerate().skip(start) {
            if trace.len() >= 30 || (i > start && i > idx && js::gs(x, "type") == "demande") {
                break;
            }
            let mark = if i == idx { "→ " } else { "  " };
            trace.push(format!("{mark}{}", Self::trace_line(x)));
        }
        let root = crate::paths::root().display().to_string();
        let file = self.write_prompt(
            &format!("commentaire-{seq}-{}.md", js::now_ms()),
            &super::failures::comment_prompt(
                &json!({
                    "seq": seq, "at": r["at"], "label": journal_label(&js::gs(r, "type")), "summary": js::slice_chars(&summarize(r), 300),
                    "comment": text,
                    "demande": req.as_ref().map(|q| js::gs(&q["data"], "texte")).unwrap_or_default(),
                    "reason": if req.is_some() { super::failures::request_outcome(&all, start)["reason"].clone() } else { json!("") },
                    "trace": trace
                }),
                &root,
            ),
        );
        let res = self.open_claude_cmd(&file, &root);
        if !js::gb(&res, "ok") {
            return res;
        }
        self.orch.record("commentaire_action", json!({ "seq": seq, "commentaire": text, "consigne": file }));
        json!({ "ok": true, "where": res["where"] })
    }

    fn write_prompt(&self, name: &str, content: &str) -> String {
        let dir = self.orch.workspace.join("apprentissage");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(name);
        let _ = std::fs::write(&file, content);
        file.display().to_string()
    }

    /// Fenêtre cmd dans le dossier de Jarvis, Claude Code en mode sans restriction avec la consigne à
    /// suivre : on suit son travail et on peut intervenir. Lancée par Start-Process pour avoir sa
    /// propre console, visible.
    fn open_claude_cmd(&self, file: &str, root: &str) -> J {
        let claude = &self.orch.claude;
        if claude.is_empty() {
            return json!({ "ok": false, "error": "Claude Code introuvable (claude.exe)" });
        }
        let exe = if claude.contains(char::is_whitespace) { format!("\"{claude}\"") } else { claude.clone() };
        let line = format!("title Jarvis - apprentissage & {exe} --dangerously-skip-permissions \"Lis le fichier {file} et applique ses instructions.\"");
        let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let ps = format!("Start-Process -FilePath 'cmd.exe' -WorkingDirectory {} -ArgumentList '/k', {}", q(root), q(&line));
        let mut cmd = std::process::Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &ps]);
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        match cmd.spawn() {
            Ok(_) => json!({ "ok": true, "where": "fenêtre cmd (Claude Code, mode sans restriction)" }),
            Err(e) => json!({ "ok": false, "error": format!("fenêtre cmd impossible : {e}") }),
        }
    }

    // --- Lecture ---------------------------------------------------------------------------------

    pub fn catalog(&self) -> J {
        let mut claude: Vec<String> = vec!["auto".into()];
        for m in status::CLAUDE_MODELS {
            if let Some(v) = status::model_of(m) {
                if !claude.contains(&v) {
                    claude.push(v);
                }
            }
        }
        let mut agy: Vec<String> = vec!["auto".into()];
        for k in status::AGY_MODEL_KEYS {
            if let Some(v) = status::agy_model(k) {
                if !agy.iter().any(|x| x == v) {
                    agy.push(v.to_string());
                }
            }
        }
        let tones: Map<String, Value> = super::config::TONE_KEYS.iter().map(|k| (k.to_string(), json!(TONE_LABELS.iter().find(|(x, _)| x == k).map(|(_, v)| *v).unwrap_or(k)))).collect();
        let verb: Map<String, Value> = super::config::VERBOSITY_KEYS.iter().map(|k| (k.to_string(), json!(VERBOSITY_LABELS.iter().find(|(x, _)| x == k).map(|(_, v)| *v).unwrap_or(k)))).collect();
        json!({
            "engines": { "claude": "Claude Code", "agy": "AGY (Antigravity)" },
            "models": { "claude": claude, "agy": agy },
            "efforts": status::EFFORT_VALUES,
            "voices": status::VOICES.iter().map(|v| json!({ "key": v.0, "id": v.1, "name": v.2, "gender": v.3, "origin": v.4 })).collect::<Vec<_>>(),
            "themes": map_of(THEMES),
            "heartStyles": map_of(status::HEART_STYLES),
            "densities": map_of(DENSITIES),
            "danceStyles": { "auto": "Automatique (selon la musique)", "calme": "Calme", "pop": "Pop", "electro": "Électro", "rock": "Rock", "rap": "Rap", "latino": "Latino" },
            "listening": map_of(LISTENING),
            "contextMute": map_of(CONTEXT_MUTE),
            "tones": tones,
            "verbosity": verb,
            "zoneLevels": { "free": "Libre", "code": "Code exigé", "forbidden": "Interdit" },
            "performancePresets": super::config::performance_presets(),
            "maxFps": super::config::MAX_FPS_CHOICES,
            "autoPerf": super::auto_perf::describe()
        })
    }

    /// Toutes les cards du HUD pour l'administration : clé, nom, type (1 ou 2), numéro actuel et numéro par défaut.
    fn cards_list(&self) -> J {
        let o = &self.orch;
        let numbers = o.status.d.borrow().card_numbers.clone();
        // (clé, nom, type, numéro par défaut)
        let known: [(&str, &str, i64, i64); 14] = [
            ("bureau", "Bureau", 1, 1), ("audio", "Audio", 1, 2), ("hw", "Matériel", 1, 3), ("conversation", "Conversation", 1, 4), ("actions", "Actions", 1, 5),
            ("music", "Musique", 2, 100), ("tool:Terminal", "Terminal", 2, 101), ("mail", "Mails", 2, 102), ("agenda", "Agenda", 2, 103),
            ("note", "Note", 2, 104), ("meeting", "Réunion", 2, 105), ("tool:Mes listes", "Mes listes", 2, 106),
            ("tool:notes", "Mes notes (liste)", 2, 107), ("tool:reunions", "Mes réunions (liste)", 2, 108),
        ];
        let open = o.cards.list();
        let mut out: Vec<J> = known
            .iter()
            .map(|(key, label, kind, def)| {
                let current = numbers[*key].as_i64().or_else(|| open.iter().find(|c| c.slot == *key).map(|c| c.n)).or(if *def > 0 { Some(*def) } else { None });
                json!({ "key": key, "label": label, "type": kind, "number": current, "custom": numbers[*key].is_i64(), "default": if *def > 0 { json!(def) } else { Value::Null }, "open": open.iter().any(|c| c.slot == *key) })
            })
            .collect();
        for c in &open {
            if !known.iter().any(|k| k.0 == c.slot) {
                out.push(json!({ "key": c.slot, "label": c.title, "type": 2, "number": numbers[c.slot.as_str()].as_i64().unwrap_or(c.n), "custom": numbers[c.slot.as_str()].is_i64(), "default": Value::Null, "open": true }));
            }
        }
        Value::Array(out)
    }

    pub fn snapshot(&self) -> J {
        let o = &self.orch;
        let integrity = o.history.verify_cached();
        let mut st = o.status.snapshot();
        {
            let d = o.status.d.borrow();
            st["model"] = json!(d.model);
            st["activeModel"] = json!(d.active_model);
            st["autostart"] = json!(d.autostart);
        }
        st["contextMute"] = o.context_mute.get_state();
        let vocab_active = o.vocabulary.build()["terms"].clone();
        let corrections: Vec<J> = o.vocabulary.corrections.borrow().iter().map(|(h, r)| json!({ "heard": h, "right": r })).collect();
        let history = o.usage_daily.history();
        let start = history.len().saturating_sub(7);
        json!({
            "status": st,
            "config": o.config.all(),
            "memory": o.memory.list(),
            "memoryAuto": { "journal": o.memory_extract.journal(), "pending": o.memory_extract.pending() },
            "announcementsPending": o.announcer.pending(),
            "presence": o.presence.state(),
            "emotions": o.emotions.list(),
            "zones": o.zones.list(),
            "vocabulary": { "learned": *o.vocabulary.terms.borrow(), "corrections": corrections, "active": vocab_active },
            "voice": self.voice.status(),
            "journal": { "count": integrity["count"], "ok": integrity["ok"], "reason": if js::truthy(&integrity["reason"]) { integrity["reason"].clone() } else { Value::Null } },
            "userName": o.memory.get_user_name(),
            "personaPreview": o.persona(),
            "speaker": o.get_speaker_status(),
            // v5 : consommation des 7 derniers jours et raccourcis personnels.
            "economy": { "history": history[start..].to_vec(), "macros": o.macros.list() },
            "catalog": self.catalog(),
            "cardsList": self.cards_list(),
            "buildSpace": o.build_space_state()
        })
    }

    // --- Reconnaissance vocale : corpus, banc d'essai, calibrage -----------------------------------

    /// action : 'phrase' (id) | 'negatif' (seconds) | 'stop'. Le moteur vocal enregistre le prochain
    /// segment de parole au lieu de le transmettre à Jarvis.
    fn corpus_record(&self, a: &J) -> J {
        let action = js::gs(a, "action");
        let action = if ["phrase", "negatif", "stop"].contains(&action.as_str()) { action } else { "stop".into() };
        let mut payload = json!({ "t": "corpus", "action": action });
        if action == "phrase" {
            let id = js::gs(a, "id");
            let Some(p) = super::voice_corpus::PHRASES.iter().find(|p| p.0 == id) else { return json!({ "ok": false, "error": "phrase inconnue" }) };
            payload["id"] = json!(p.0);
            payload["text"] = json!(p.1);
        } else if action == "negatif" {
            payload["seconds"] = json!(js::gn(a, "seconds").filter(|s| *s != 0.0).unwrap_or(60.0).clamp(10.0, 300.0) as i64);
        }
        self.orch.emit(json!({ "type": "voice_command", "payload": payload }));
        json!({ "ok": true })
    }

    /// Banc d'essai lancé en tâche de fond (jarvis-voice bench) : une seule mesure à la fois.
    fn bench_run(&self, a: &J) -> J {
        if self.bench_state.borrow().as_ref().map(|b| js::gb(b, "running")).unwrap_or(false) {
            return json!({ "ok": false, "error": "une mesure est déjà en cours" });
        }
        let kind = if js::gs(a, "kind") == "speaker" { "speaker" } else { "stt" };
        let exe = crate::paths::bin_dir().join("jarvis-voice.exe");
        let mut args: Vec<String> = vec!["bench".into(), kind.into(), "--data".into(), self.data_dir().display().to_string()];
        if kind == "stt" {
            let models = js::str_list(&a["models"]);
            if !models.is_empty() {
                args.extend(["--models".into(), models.join(",")]);
            }
        }
        if kind == "speaker" && js::truthy(&a["synthetic"]) {
            args.push("--voix-synthese".into());
        }
        *self.bench_state.borrow_mut() = Some(json!({ "running": true, "kind": kind, "startedAt": js::now_ms(), "log": [] }));
        let push = |state: Rc<RefCell<Option<J>>>| {
            move |line: String| {
                if let Some(s) = state.borrow_mut().as_mut() {
                    let mut log = js::str_list(&s["log"]);
                    log.push(line);
                    let start = log.len().saturating_sub(40);
                    s["log"] = json!(log[start..]);
                }
            }
        };
        let (st_exit, orch) = (self.bench_state.clone(), self.orch.clone());
        let res = crate::proc::LineChild::spawn_in(
            Some(&crate::paths::root()),
            &exe,
            &args,
            &[],
            push(self.bench_state.clone()),
            push(self.bench_state.clone()),
            move |code| {
                if let Some(s) = st_exit.borrow_mut().as_mut() {
                    s["running"] = json!(false);
                    s["code"] = json!(code);
                }
                orch.record("banc_reconnaissance", json!({ "type": kind, "code": code }));
            },
        );
        match res {
            Ok(_) => json!({ "ok": true }),
            Err(e) => {
                if let Some(s) = self.bench_state.borrow_mut().as_mut() {
                    s["running"] = json!(false);
                    s["log"] = json!([format!("lancement impossible : {e}")]);
                }
                json!({ "ok": false, "error": e })
            }
        }
    }

    /// Tout ce dont l'assistant de mise en place de la voix a besoin, en un appel.
    async fn voice_setup_state(&self) -> J {
        let o = &self.orch;
        let audio = super::system::get_audio_device().await;
        let lk = &o.config.get("voice")["livekit"];
        let mode = js::gs(&audio, "mode");
        json!({
            "ok": true,
            "speaker": o.get_speaker_status(),
            "userName": o.memory.get_user_name(),
            "audio": { "mode": if mode.is_empty() { Value::Null } else if mode == "casque" { json!("casque") } else { json!("haut-parleurs") }, "raw": audio },
            "corpus": super::voice_corpus::corpus_state(&self.data_dir()),
            "bench": *self.bench_state.borrow(),
            "sttModel": Some(js::gs(lk, "sttModel")).filter(|s| !s.is_empty()).unwrap_or_else(|| "turbo".into())
        })
    }

    /// Bascule casque ↔ micro et haut-parleurs du PC.
    async fn audio_switch(&self, mode: &str) -> J {
        let target = if mode == "casque" { "casque" } else { "pc" };
        let res = super::system::set_audio_device(target).await;
        let mut ev = json!({ "type": "audio_device" });
        js::assign(&mut ev, res.clone());
        self.orch.emit(ev);
        self.orch.record("bascule_audio", json!({ "mode": target, "origine": "assistant voix" }));
        spread(json!({ "ok": js::truthy(&res["mode"]) }), res)
    }

    /// Seuils du filtre de locuteur recommandés par le calibrage, appliqués à chaud.
    fn speaker_apply_thresholds(&self, t: &J) -> J {
        let mut clean = Map::new();
        for k in ["court", "moyen", "long"] {
            if let Some(v) = js::num(&t[k]).filter(|v| v.is_finite()) {
                clean.insert(k.into(), json!(v.clamp(0.4, 0.9)));
            }
        }
        if clean.is_empty() {
            return json!({ "ok": false, "error": "seuils invalides" });
        }
        let clean = J::Object(clean);
        self.orch.emit(json!({ "type": "voice_command", "payload": { "t": "speaker_set_thresholds", "thresholds": clean } }));
        self.orch.record("changement_seuil_locuteur", json!({ "seuils": clean, "origine": "calibrage" }));
        json!({ "ok": true, "thresholds": clean })
    }

    // --- Réglages de session (moteur, modèle, voix, thème…) ---------------------------------

    async fn set_status(&self, patch: &J) -> J {
        let o = &self.orch;
        let mut applied: Vec<String> = vec![];
        macro_rules! run {
            ($cmd:expr) => {{
                let c: J = $cmd;
                applied.push(js::gs(&c, "kind"));
                o.apply_command(c, true).await;
            }};
        }
        let d = || o.status.d.borrow();
        if patch.get("aiChain").is_some() {
            let (cur, eng, mdl) = {
                let st = d();
                (st.ai_chain.clone(), st.engine.clone(), st.model.clone())
            };
            let next = status::sanitize_ai_chain(&patch["aiChain"], &eng, mdl.as_deref());
            if next != cur {
                run!(json!({ "kind": "ai_chain", "value": next }));
            }
        }
        let engine = js::gs(patch, "engine");
        if (engine == "claude" || engine == "agy") && engine != d().engine {
            run!(json!({ "kind": "engine", "value": engine }));
        }
        if patch.get("model").is_some() && !patch["model"].is_null() {
            let value = js::s(&patch["model"]);
            if Some(value.clone()) != d().model {
                let is_agy = status::is_agy_model_key(&value);
                let known = value == "auto" || status::CLAUDE_MODELS.contains(&value.as_str()) || is_agy;
                if known {
                    let eng = if value == "auto" { d().engine.clone() } else if is_agy { "agy".into() } else { "claude".into() };
                    run!(json!({ "kind": "model", "value": value, "engine": eng }));
                }
            }
        }
        let effort = js::gs(patch, "effort");
        if status::EFFORT_VALUES.contains(&effort.as_str()) && Some(effort.clone()) != d().effort {
            run!(json!({ "kind": "effort", "value": effort }));
        }
        if patch.get("economyMode").is_some() && js::truthy(&patch["economyMode"]) != d().economy_mode {
            run!(json!({ "kind": "economy", "value": js::truthy(&patch["economyMode"]) }));
        }
        let level = js::gs(patch, "economyLevel");
        if ["normal", "local", "strict"].contains(&level.as_str()) && level != d().economy_level {
            run!(json!({ "kind": "economy_level", "value": level }));
        }
        if js::truthy(&patch["voice"]) {
            if let Some(v) = status::resolve_voice(&js::s(&patch["voice"])) {
                if js::gs(&v, "id") != d().voice {
                    run!(json!({ "kind": "voice", "value": v }));
                }
            }
        }
        let rate = js::gs(patch, "voiceRate");
        if crate::re!(r"^[+-]\d{1,3}%$").is_match(&rate) && rate != d().voice_rate {
            run!(json!({ "kind": "rate", "value": rate }));
        }
        let lm = js::gs(patch, "listeningMode");
        if has_key(LISTENING, &lm) && lm != d().listening_mode {
            run!(json!({ "kind": "listening_mode", "value": lm }));
        }
        let theme = js::gs(patch, "theme");
        if has_key(THEMES, &theme) && theme != d().theme {
            run!(json!({ "kind": "theme", "value": theme }));
        }
        let hs = js::gs(patch, "heartStyle");
        if status::heart_style_label(&hs).is_some() && hs != d().heart_style {
            run!(json!({ "kind": "heart_style", "value": hs }));
        }
        if patch.get("characterVoice").is_some() && js::truthy(&patch["characterVoice"]) != d().character_voice {
            run!(json!({ "kind": "character_voice", "value": js::truthy(&patch["characterVoice"]) }));
        }
        if patch.get("characterPersona").is_some() && js::truthy(&patch["characterPersona"]) != d().character_persona {
            run!(json!({ "kind": "character_persona", "value": js::truthy(&patch["characterPersona"]) }));
        }
        let density = js::gs(patch, "density");
        if has_key(DENSITIES, &density) && density != d().density {
            run!(json!({ "kind": "density", "value": density }));
        }
        if patch.get("cardColors").is_some() && status::sanitize_card_colors(&patch["cardColors"]) != d().card_colors {
            run!(json!({ "kind": "card_colors", "value": patch["cardColors"].clone() }));
        }
        let dance = js::gs(patch, "danceStyle");
        if status::DANCE_STYLES.contains(&dance.as_str()) && dance != d().dance_style {
            run!(json!({ "kind": "dance_style", "value": dance }));
        }
        if patch.get("cardNumbers").is_some() && status::sanitize_card_numbers(&patch["cardNumbers"]) != d().card_numbers {
            run!(json!({ "kind": "card_numbers", "value": patch["cardNumbers"].clone() }));
        }
        let cm = js::gs(patch, "contextMute");
        if has_key(CONTEXT_MUTE, &cm) {
            run!(json!({ "kind": "context_mute", "value": cm }));
        }
        if patch.get("autostart").is_some() && js::truthy(&patch["autostart"]) != d().autostart {
            run!(json!({ "kind": "autostart", "value": js::truthy(&patch["autostart"]) }));
        }
        if !applied.is_empty() {
            o.refresh_daemon();
        }
        json!({ "ok": true, "applied": applied, "snapshot": self.snapshot() })
    }

    /// Aperçu de voix : Jarvis prononce une phrase d'exemple.
    fn preview_voice(&self) -> J {
        let speech = self.orch.status.speech();
        let mut ev = json!({ "type": "voice_config" });
        js::assign(&mut ev, speech.clone());
        ev["sample"] = json!(true);
        if js::truthy(&speech["sample"]) {
            ev["sampleText"] = speech["sample"].clone();
        }
        self.orch.emit(ev);
        json!({ "ok": true })
    }

    fn set_config(&self, patch: &J) -> J {
        let o = &self.orch;
        let changed = o.config.update(patch);
        if changed.iter().any(|c| c == "profile") {
            o.sync_profile_memory();
            o.publish_memory();
            o.refresh_daemon(); // la consigne système change : le démon repart avec le nouveau profil
        }
        if changed.iter().any(|c| c == "performance") {
            o.apply_performance();
        }
        // Présence : le HUD démarre ou arrête la capture aussitôt.
        if changed.iter().any(|c| c == "presence") {
            if !o.presence.enabled() {
                o.presence.reset();
            }
            o.emit(json!({ "type": "presence_config", "config": o.presence.hud_config() }));
        }
        // Mémoire automatique coupée : les phrases en attente sont oubliées sans être envoyées.
        if changed.iter().any(|c| c == "memoryAuto") && o.config.get("memoryAuto")["enabled"] != json!(true) {
            o.memory_extract.disable();
        }
        if changed.iter().any(|c| c == "visio") {
            let o2 = o.clone();
            js::spawn(async move {
                o2.context_mute.refresh_meet(true).await;
                o2.publish_status();
            });
        }
        if !changed.is_empty() {
            o.record("reglages_modifies", json!({ "sections": changed, "origine": "administration" }));
            (self.on_config_change)(&changed, &o.config.all());
        }
        json!({ "ok": true, "changed": changed, "snapshot": self.snapshot() })
    }

    // --- Optimisation : consommation réelle ---------------------------------------------------

    fn perf_metrics(&self) -> J {
        let metrics = (self.get_metrics)();
        let voice: Vec<J> = self
            .voice
            .pid()
            .map(|pid| {
                let mb = jarvis_native::window::process_stats(pid).map(|(ws, _)| json!(ws / 1_048_576)).unwrap_or(Value::Null);
                vec![json!({ "label": "Moteur vocal (Rust, Whisper)", "mb": mb })]
            })
            .unwrap_or_default();
        json!({
            "ok": true, "electron": metrics, "voice": voice,
            "voicePaused": self.voice.paused.get(),
            "semanticLoaded": self.orch.embedder.as_ref().map(|e| e.active()).unwrap_or(false),
            "auto": self.orch.auto_perf_info()
        })
    }

    // --- Mémoire (RAG) --------------------------------------------------------------------------

    fn after_memory_change(&self) {
        let o = &self.orch;
        o.publish_memory();
        o.refresh_daemon();
        o.publish_vocabulary(false);
        let o2 = o.clone();
        js::spawn(async move {
            let _ = o2.memory.index().await;
        });
    }

    fn memory_add(&self, text: &str, category: &str, expires: &str) -> J {
        let expires = match memory::parse_expiry(expires) {
            Ok(d) => d,
            Err(e) => return json!({ "ok": false, "error": e }),
        };
        let Some(entry) = self.orch.memory.add_until(text, if category.is_empty() { "divers" } else { category }, expires.as_deref()) else { return json!({ "ok": false, "error": "texte vide" }) };
        self.orch.record("memoire_ajout", json!({ "categorie": entry["category"], "texte": entry["text"], "expire": entry["expires"], "origine": "administration" }));
        self.after_memory_change();
        json!({ "ok": true, "entry": entry, "snapshot": self.snapshot() })
    }

    /// `expires` : None = échéance inchangée ; Some("") = souvenir rendu durable.
    fn memory_update(&self, id: &str, text: &str, category: &str, expires: Option<String>) -> J {
        let Some(target) = self.orch.memory.list().into_iter().find(|e| js::gs(e, "id") == id) else { return json!({ "ok": false, "error": "souvenir introuvable" }) };
        if js::truthy(&target["pinned"]) {
            return json!({ "ok": false, "error": "ce souvenir vient du profil de dialogue : modifie-le dans « Profil & préférences »" });
        }
        let before = js::gs(&target, "text");
        // Une échéance gardée telle quelle n'a pas à être dans le futur (elle peut échoir aujourd'hui).
        let expires = match expires {
            Some(raw) if raw.trim() != js::gs(&target, "expires") => match memory::parse_expiry(&raw) {
                Ok(d) => Some(d),
                Err(e) => return json!({ "ok": false, "error": e }),
            },
            _ => None,
        };
        let Some(mut entry) = self.orch.memory.update(id, Some(text), (!category.is_empty()).then_some(category)) else { return json!({ "ok": false, "error": "texte vide" }) };
        if let Some(d) = expires {
            entry = self.orch.memory.set_expiry(id, d.as_deref()).unwrap_or(entry);
        }
        self.orch.record("memoire_modification", json!({ "avant": before, "apres": entry["text"], "categorie": entry["category"], "origine": "administration" }));
        self.after_memory_change();
        json!({ "ok": true, "entry": entry, "snapshot": self.snapshot() })
    }

    fn memory_delete(&self, id: &str) -> J {
        if self.orch.memory.list().iter().any(|e| js::gs(e, "id") == id && js::truthy(&e["pinned"])) {
            return json!({ "ok": false, "error": "ce souvenir vient du profil de dialogue" });
        }
        let Some(entry) = self.orch.memory.forget(id) else { return json!({ "ok": false, "error": "souvenir introuvable" }) };
        self.orch.record("memoire_oubli", json!({ "categorie": entry["category"], "texte": entry["text"], "origine": "administration" }));
        self.after_memory_change();
        json!({ "ok": true, "entry": entry, "snapshot": self.snapshot() })
    }

    fn memory_purge_confirm(&self, id: &str, code: &str) -> J {
        if let Err(e) = self.consume(id, code, "memory_purge") {
            return e;
        }
        let result = self.orch.clear_memory();
        json!({ "ok": true, "deleted": result["deleted"], "snapshot": self.snapshot() })
    }

    // --- Zones d'accès et vocabulaire ---------------------------------------------------------

    fn zone_add(&self, name: &str, target: &str, read: &str, write: &str) -> J {
        let clean = name.trim().to_string();
        if clean.is_empty() || target.is_empty() {
            return json!({ "ok": false, "error": "nom et chemin requis" });
        }
        if !Path::new(target).is_absolute() {
            return json!({ "ok": false, "error": "chemin absolu requis" });
        }
        // Une zone ne peut jamais couvrir les données internes de Jarvis.
        let data = super::zones::resolve(&self.data_dir().display().to_string()).to_lowercase();
        let resolved = super::zones::resolve(target).to_lowercase();
        if resolved == data || resolved.starts_with(&format!("{data}\\")) || data.starts_with(&format!("{resolved}\\")) {
            return json!({ "ok": false, "error": "une zone ne peut pas contenir les données internes de Jarvis" });
        }
        let payload = json!({ "name": clean, "targetPath": target, "read": read, "write": write });
        // Ouvrir l'écriture libre sur une zone abaisse la sécurité : code exigé.
        if write == "free" {
            return self.challenge("zone_add", payload, &format!("Écriture libre dans « {clean} »"));
        }
        self.apply_zone_add(&payload)
    }

    fn apply_zone_add(&self, p: &J) -> J {
        let entry = self.orch.zones.add(&js::gs(p, "name"), &js::gs(p, "targetPath"), &js::gs(p, "read"), &js::gs(p, "write"));
        self.orch.record("zone_ajoutee", json!({ "nom": entry["name"], "chemin": entry["path"], "lecture": entry["read"], "ecriture": entry["write"], "origine": "administration" }));
        self.orch.refresh_daemon();
        self.orch.publish_vocabulary(false);
        json!({ "ok": true, "entry": entry, "snapshot": self.snapshot() })
    }

    fn zone_remove(&self, id: &str) -> J {
        let Some(removed) = self.orch.zones.remove(id) else { return json!({ "ok": false, "error": "zone introuvable ou protégée" }) };
        self.orch.record("zone_supprimee", json!({ "nom": removed["name"], "chemin": removed["path"], "origine": "administration" }));
        self.orch.refresh_daemon();
        self.orch.publish_vocabulary(false);
        json!({ "ok": true, "snapshot": self.snapshot() })
    }

    fn vocab_add(&self, term: &str) -> J {
        let Some(added) = self.orch.vocabulary.add(term) else { return json!({ "ok": false, "error": "mot vide ou trop long" }) };
        self.orch.record("vocabulaire_ajout", json!({ "terme": added, "origine": "administration" }));
        self.orch.publish_vocabulary(false);
        json!({ "ok": true, "snapshot": self.snapshot() })
    }

    fn vocab_remove(&self, term: &str) -> J {
        if !self.orch.vocabulary.remove(term) {
            return json!({ "ok": false, "error": "mot introuvable" });
        }
        self.orch.record("vocabulaire_retrait", json!({ "terme": term, "origine": "administration" }));
        self.orch.publish_vocabulary(false);
        json!({ "ok": true, "snapshot": self.snapshot() })
    }

    fn vocab_correct(&self, heard: &str, right: &str) -> J {
        let Some(pair) = self.orch.vocabulary.correct(heard, right) else { return json!({ "ok": false, "error": "correction invalide" }) };
        self.orch.record("vocabulaire_correction", spread(pair, json!({ "origine": "administration" })));
        self.orch.publish_vocabulary(false);
        json!({ "ok": true, "snapshot": self.snapshot() })
    }

    // --- Journal & dialogues -------------------------------------------------------------------

    /// view : 'dialogues' (conversation) | 'actions' (tout). Plus récentes d'abord, par pages.
    fn journal_query(&self, q: &J) -> J {
        let view = Some(js::gs(q, "view")).filter(|v| !v.is_empty()).unwrap_or_else(|| "actions".into());
        let types = js::str_list(&q["types"]);
        let wanted: Option<Vec<String>> = if view == "dialogues" {
            Some(DIALOGUE_TYPES.iter().map(|s| s.to_string()).collect())
        } else if !types.is_empty() {
            Some(types)
        } else {
            None
        };
        let limit = js::gn(q, "limit").filter(|l| *l > 0.0).unwrap_or(120.0).clamp(1.0, 500.0) as usize;
        let before = js::gn(q, "before").map(|b| b as i64);
        let rows = self.orch.history.query(wanted.as_deref(), &js::gs(q, "text"), before, limit);
        // Issue de chaque demande et demandes déjà confiées à Claude Code.
        let all = self.orch.history.read(false);
        let pos: HashMap<i64, usize> = all.iter().enumerate().filter_map(|(i, r)| r["seq"].as_i64().map(|s| (s, i))).collect();
        let asked: HashSet<i64> = all.iter().filter(|r| js::gs(r, "type") == "apprentissage_demande").filter_map(|r| js::num(&r["data"]["seq"]).map(|s| s as i64)).collect();
        let mut comments: HashMap<i64, Vec<J>> = HashMap::new();
        for r in &all {
            if js::gs(r, "type") != "commentaire_action" || !r["data"].is_object() {
                continue;
            }
            if let Some(s) = js::num(&r["data"]["seq"]) {
                comments.entry(s as i64).or_default().push(json!({ "at": r["at"], "text": r["data"]["commentaire"] }));
            }
        }
        let entries: Vec<J> = rows
            .iter()
            .map(|r| {
                let seq = r["seq"].as_i64().unwrap_or(-1);
                let typ = js::gs(r, "type");
                let mut e = json!({
                    "seq": r["seq"], "at": r["at"], "type": typ, "label": journal_label(&typ),
                    "summary": summarize(r), "data": r["data"], "hash": js::slice_chars(&js::gs(r, "hash"), 12)
                });
                if let Some(c) = comments.get(&seq) {
                    e["comments"] = json!(c);
                }
                if typ == "demande" {
                    if let Some(i) = pos.get(&seq) {
                        e["outcome"] = super::failures::request_outcome(&all, *i);
                        e["learning"] = json!(asked.contains(&seq));
                    }
                }
                e
            })
            .collect();
        json!({ "ok": true, "more": entries.len() >= limit, "entries": entries })
    }

    fn journal_stats(&self) -> J {
        let counts = self.orch.history.counts();
        let mut list: Vec<(String, i64)> = counts.as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0))).collect()).unwrap_or_default();
        list.sort_by(|a, b| b.1.cmp(&a.1));
        json!({ "ok": true, "types": list.iter().map(|(t, c)| json!({ "type": t, "count": c, "label": journal_label(t) })).collect::<Vec<_>>() })
    }

    // --- Sécurité ----------------------------------------------------------------------------------

    fn level_labels() -> J {
        J::Object(super::security::LEVELS.iter().map(|l| (l.to_string(), json!(super::security::level_label(l)))).collect())
    }

    fn security_list(&self) -> J {
        let o = &self.orch;
        let types: Vec<String> = ["validation_demandee", "validation_accordee", "validation_refusee", "validation_expiree", "acces_refuse", "securite_modifiee"].iter().map(|s| s.to_string()).collect();
        let recent: Vec<J> = o
            .history
            .query(Some(&types), "", None, 40)
            .iter()
            .map(|r| json!({ "seq": r["seq"], "at": r["at"], "type": r["type"], "label": journal_label(&js::gs(r, "type")), "summary": summarize(r) }))
            .collect();
        json!({ "ok": true, "entries": o.security.list(), "levels": Self::level_labels(), "pending": o.pending_list(), "recent": recent })
    }

    /// Resserrer est immédiat ; abaisser (vers l'accord direct notamment) exige le code.
    fn security_set(&self, key: &str, level: &str) -> J {
        let sec = &self.orch.security;
        let Some(entry) = sec.entry(key) else { return json!({ "ok": false, "error": "type d'action inconnu" }) };
        if !super::security::LEVELS.contains(&level) {
            return json!({ "ok": false, "error": "niveau inconnu" });
        }
        if sec.level_for(key) == level {
            return json!({ "ok": true, "unchanged": true });
        }
        if sec.is_lowering(key, level) {
            let label = format!("Abaisser « {} » à : {}", js::gs(&entry, "label"), super::security::level_label(level));
            return self.challenge("security_lower", json!({ "key": key, "level": level }), &label);
        }
        self.apply_security(key, level)
    }

    fn apply_security(&self, key: &str, level: &str) -> J {
        match self.orch.security.set(key, level) {
            Ok(change) => {
                self.orch.record("securite_modifiee", json!({ "cle": key, "avant": change["before"], "apres": change["after"], "origine": "administration" }));
                self.orch.emit(json!({ "type": "security", "pending": self.orch.pending_list() }));
                json!({ "ok": true, "change": change, "list": self.security_list() })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    // --- Gmail intégré (IMAP / SMTP, mot de passe d'application) -------------------------------------

    fn gmail_state(&self) -> J {
        let g = &self.orch.google;
        let email = js::gs(&g.config.borrow(), "email");
        json!({
            "configured": g.is_configured(),
            "email": email,
            "encrypted": g.secret_box_available(),
            "masked": if email.is_empty() { String::new() } else { mask_email(&email) }
        })
    }

    /// Vérifie l'accès réel (connexion IMAP) avant d'enregistrer : un identifiant faux n'est jamais gardé.
    async fn gmail_configure(&self, email: &str, app_password: &str) -> J {
        let g = &self.orch.google;
        let clean = email.trim().to_string();
        let typed = app_password.trim().to_string();
        if !crate::re!(r"(?i)^[^@\s]+@[^@\s]+\.[a-z]{2,}$").is_match(&clean) {
            return json!({ "ok": false, "error": "adresse email invalide" });
        }
        let previous = g.config.borrow().clone();
        let password0 = if !typed.is_empty() {
            typed
        } else if clean == js::gs(&previous, "email") {
            js::gs(&previous, "appPassword")
        } else {
            String::new()
        };
        if password0.is_empty() {
            return json!({ "ok": false, "error": "mot de passe d’application requis (16 lettres)" });
        }
        // Pas un mot de passe d'application : inutile de le soumettre à Google.
        let password = match super::google::check_app_password(&password0) {
            Ok(p) => p,
            Err(text) => {
                self.orch.record("gmail_configuration", json!({ "email": mask_email(&clean), "ok": false, "erreur": "format de mot de passe d’application invalide (non envoyé à Google)", "origine": "administration" }));
                return json!({ "ok": false, "auth": true, "format": true, "error": text });
            }
        };
        {
            let mut c = g.config.borrow_mut();
            c["email"] = json!(clean);
            c["appPassword"] = json!(password);
        }
        let test = g.mail_derniers(&json!({ "limite": 3 })).await;
        if !js::gb(&test, "ok") {
            *g.config.borrow_mut() = previous;
            let err = if js::truthy(&test["error"]) { js::gs(&test, "error") } else { js::gs(&test, "message") };
            self.orch.record("gmail_configuration", json!({ "email": mask_email(&clean), "ok": false, "erreur": err, "origine": "administration" }));
            let (auth, text) = super::google::explain_gmail_error(&err);
            return json!({ "ok": false, "auth": auth, "error": if auth { text } else { err } });
        }
        g.save_config(json!({ "email": clean, "appPassword": password }));
        self.orch.record("gmail_configuration", json!({ "email": mask_email(&clean), "ok": true, "origine": "administration" }));
        let sample: Vec<J> = js::arr(&test["mails"]).into_iter().take(3).collect();
        json!({ "ok": true, "count": test["nombre"], "sample": sample, "gmail": self.gmail_state() })
    }

    async fn gmail_test(&self) -> J {
        let test = self.orch.google.mail_derniers(&json!({ "limite": 3 })).await;
        if !js::gb(&test, "ok") {
            let err = if js::truthy(&test["error"]) { js::gs(&test, "error") } else { js::gs(&test, "message") };
            let (auth, text) = super::google::explain_gmail_error(&err);
            return json!({ "ok": false, "auth": auth, "error": if auth { text } else { err } });
        }
        let sample: Vec<J> = js::arr(&test["mails"]).into_iter().take(3).collect();
        json!({ "ok": true, "count": test["nombre"], "sample": sample, "gmail": self.gmail_state() })
    }

    // --- Prononciation -----------------------------------------------------------------------------

    fn pronunciation_state(&self) -> J {
        let builtin: Vec<J> = super::pronunciation::BUILTIN.iter().map(|(w, s)| json!({ "word": w, "say": s })).collect();
        let voice = self.orch.status.d.borrow().voice.clone();
        let voices: Vec<J> = super::status::VOICES.iter().map(|v| json!({ "id": v.1, "name": v.2 })).collect();
        json!({ "ok": true, "entries": self.orch.pronunciation.list(), "builtin": builtin, "voice": voice, "voices": voices })
    }

    // --- Projets ---------------------------------------------------------------------------------

    async fn projects_state(&self) -> J {
        let pr = &self.orch.projects;
        let root = pr.root();
        let exists = Path::new(&root).is_dir();
        let zone = self.orch.zones.find_zone(&root);
        let mut items = vec![];
        if exists {
            for p in pr.list() {
                items.push(pr.info(&p).await);
            }
        }
        items.sort_by(|a, b| js::gn_or(b, "lastActivity", 0.0).partial_cmp(&js::gn_or(a, "lastActivity", 0.0)).unwrap_or(std::cmp::Ordering::Equal));
        let projects: Vec<J> = items
            .into_iter()
            .map(|mut i| {
                let la = js::gn_or(&i, "lastActivity", 0.0) as i64;
                i["lastActivity"] = if la > 0 { json!(js::iso_of(la)) } else { Value::Null };
                i
            })
            .collect();
        json!({
            "ok": true, "root": root, "exists": exists,
            "zone": zone.map(|z| json!({ "name": z["name"], "read": z["read"], "write": z["write"] })),
            "projects": projects
        })
    }

    /// Nouveau dossier des projets : il doit exister. Claude Code reçoit la nouvelle liste.
    async fn projects_set_root(&self, root: &str) -> J {
        let t = crate::re!(r#"^"+|"+$"#).replace_all(root.trim(), "").to_string();
        let clean = crate::re!(r"[\\/]+$").replace(&t, "").to_string();
        if clean.is_empty() || !Path::new(&clean).is_absolute() {
            return json!({ "ok": false, "error": "chemin absolu attendu (ex. E:\\www\\A-En cours)" });
        }
        if !Path::new(&clean).is_dir() {
            return json!({ "ok": false, "error": format!("dossier introuvable : {clean}") });
        }
        self.orch.config.update(&json!({ "projects": { "root": clean } }));
        self.orch.projects.invalidate();
        self.orch.refresh_daemon();
        self.orch.record("reglages_modifies", json!({ "projets": clean, "origine": "administration" }));
        self.projects_state().await
    }

    /// Actions sur un projet de la liste (jamais sur un chemin quelconque).
    fn project_by_path(&self, target: &str) -> Option<J> {
        let want = super::zones::resolve(target).to_lowercase();
        self.orch.projects.list().into_iter().find(|p| super::zones::resolve(&js::gs(p, "path")).to_lowercase() == want)
    }

    async fn project_open(&self, target: &str) -> J {
        let Some(p) = self.project_by_path(target) else { return json!({ "ok": false, "error": "projet inconnu" }) };
        let _ = super::system::call("ouvrir_dossier", &json!({ "chemin": p["path"] })).await;
        json!({ "ok": true })
    }

    async fn project_claude(&self, target: &str) -> J {
        let Some(p) = self.project_by_path(target) else { return json!({ "ok": false, "error": "projet inconnu" }) };
        let path = js::gs(&p, "path");
        let (r, _name, command) = self.orch.tmux_voice.launch_claude(Some(&path)).await;
        if js::gb(&r, "ok") {
            return json!({ "ok": true, "where": "tmux" });
        }
        if js::gb(&r, "denied") {
            return json!({ "ok": false, "error": "lancement refusé" });
        }
        super::projects::open_claude_window(&path, &js::gs(&p, "name"), &command);
        self.orch.record("action_locale", json!({ "outil": "claude_terminal", "detail": path, "origine": "administration" }));
        json!({ "ok": true, "where": "terminal" })
    }

    /// Pages d'aide ouvertes dans le navigateur : liste fermée.
    async fn open_help_link(&self, key: &str) -> J {
        let url = match key {
            "google_app_passwords" => "https://myaccount.google.com/apppasswords",
            "google_two_step" => "https://myaccount.google.com/signinoptions/twosv",
            "google_drive_api" => "https://console.cloud.google.com/apis/library/drive.googleapis.com",
            "google_consent" => "https://console.cloud.google.com/auth/audience",
            "google_credentials" => "https://console.cloud.google.com/apis/credentials",
            _ => return json!({ "ok": false, "error": "lien inconnu" }),
        };
        let _ = super::system::call("ouvrir_url", &json!({ "url": url })).await;
        json!({ "ok": true })
    }

    /// Ouvre dans le navigateur l'aperçu de toutes les animations de chaque cœur.
    async fn open_heart_preview(&self, style: &J) -> J {
        let s = js::s(style);
        let query = if s.is_empty() { String::new() } else { format!("?style={}", urlencoding::encode(&s)) };
        let url = format!("http://127.0.0.1:{}/tools/character-preview.html{query}", self.gateway_port);
        let _ = super::system::call("ouvrir_url", &json!({ "url": url })).await;
        json!({ "ok": true, "url": url })
    }

    // --- Spotify intégré ---------------------------------------------------------------------------

    fn spotify_state(&self) -> J {
        let sp = &self.orch.spotify;
        spread(sp.state(), json!({ "installed": sp.installed_path().is_some() }))
    }

    /// Identifiants vérifiés auprès de Spotify AVANT d'être enregistrés. Erreur 403 (compte gratuit,
    /// restrictions de l'API) : le contrôle direct de l'application Windows est activé.
    async fn spotify_configure(&self, client_id: &str, client_secret: &str, options: &J) -> J {
        let sp = &self.orch.spotify;
        let id = client_id.trim().to_string();
        let secret = client_secret.trim().to_string();
        if !crate::re!(r"(?i)^[a-f0-9]{16,64}$").is_match(&id) {
            return json!({ "ok": false, "error": "client ID invalide (32 caractères, visibles sur developer.spotify.com)" });
        }
        let previous = sp.config.borrow().clone();
        let used = if !secret.is_empty() {
            secret
        } else if id == js::gs(&previous, "clientId") {
            js::gs(&previous, "clientSecret")
        } else {
            String::new()
        };
        if used.is_empty() {
            return json!({ "ok": false, "error": "client secret requis" });
        }
        {
            let mut c = sp.config.borrow_mut();
            c["clientId"] = json!(id);
            c["clientSecret"] = json!(used);
        }
        *sp.token.borrow_mut() = None;
        let test = sp.test().await;
        if !js::gb(&test, "ok") {
            let msg = js::gs(&test, "message");
            let is403 = js::gs(&test, "step") == "search" || crate::re!(r"(?i)403").is_match(&msg) || js::truthy(&test["tokenOk"]);
            if is403 || js::truthy(&options["force"]) {
                sp.config.borrow_mut()["mode"] = json!("direct");
                sp.premium_required.set(true);
                sp.save_config(json!({ "clientId": id, "clientSecret": used, "mode": "direct" }));
                self.orch.record("spotify_configuration", json!({ "ok": true, "mode": "direct", "autoFallback": is403, "avertissement": msg, "origine": "administration" }));
                return json!({
                    "ok": true, "autoFallback": is403, "mode": "direct",
                    "message": if is403 {
                        "Erreur 403 détectée sur l’API Spotify : la Solution A (Contrôle direct de l’application Windows) a été activée automatiquement ! Vos clés sont enregistrées et Jarvis pilote directement votre application Spotify.".to_string()
                    } else {
                        msg
                    },
                    "spotify": self.spotify_state()
                });
            }
            *sp.config.borrow_mut() = previous;
            *sp.token.borrow_mut() = None;
            self.orch.record("spotify_configuration", json!({ "ok": false, "erreur": msg, "origine": "administration" }));
            return json!({ "ok": false, "error": msg, "step": test["step"], "tokenOk": test["tokenOk"], "canForce": true });
        }
        sp.save_config(json!({ "clientId": id, "clientSecret": used, "mode": "api" }));
        self.orch.record("spotify_configuration", json!({ "ok": true, "mode": "api", "origine": "administration" }));
        json!({ "ok": true, "message": test["message"], "spotify": self.spotify_state() })
    }

    fn spotify_test_local(&self) -> J {
        let sp = &self.orch.spotify;
        if sp.installed_path().is_none() {
            return json!({ "ok": false, "message": "L’application Spotify n’est pas installée sur ce PC." });
        }
        let running = sp.is_running();
        json!({
            "ok": true, "installed": true, "running": running,
            "message": if running {
                "Application Spotify active sur ce PC : le contrôle direct Windows (Play, Pause, Suivant, Recherche vocale) fonctionne sans aucune clé API."
            } else {
                "Application Spotify installée sur ce PC (fermée pour l’instant) : Jarvis peut la lancer et la contrôler directement."
            }
        })
    }

    // --- Connecteurs MCP ---------------------------------------------------------------------------

    fn mcp_state(&self) -> J {
        let reg = &self.orch.mcp_registry;
        let servers: Vec<J> = reg
            .list()
            .iter()
            .map(|s| {
                let id = js::gs(s, "id");
                let missing = reg.get(&id).map(|full| reg.missing_required(&full)).unwrap_or_default();
                spread(s.clone(), json!({ "missing": missing }))
            })
            .collect();
        json!({
            "ok": true,
            "gmail": self.gmail_state(),
            "gdocs": self.orch.google_docs.state(),
            "spotify": self.spotify_state(),
            "presets": reg.presets_list(),
            "servers": servers,
            "encryption": reg.encryption()
        })
    }

    fn mcp_add(&self, def: &J) -> J {
        let def = if def.is_object() { def.clone() } else { json!({}) };
        match self.orch.mcp_registry.add(&def) {
            Ok(server) => {
                let stdio = js::gs(&server, "transport") == "stdio";
                let commande = if stdio { format!("{} {}", js::gs(&server, "command"), js::str_list(&server["args"]).join(" ")) } else { js::gs(&server, "url") };
                self.orch.record(
                    "mcp_ajoute",
                    json!({ "id": server["id"], "nom": server["name"], "modele": server["preset"], "transport": server["transport"], "commande": commande, "origine": "administration" }),
                );
                self.orch.refresh_connectors();
                spread(self.mcp_state(), json!({ "added": server["id"] }))
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    fn mcp_update(&self, id: &str, patch: &J) -> J {
        let patch = if patch.is_object() { patch.clone() } else { json!({}) };
        match self.orch.mcp_registry.update(id, &patch) {
            Ok(server) => {
                // Les valeurs d'environnement ne sont jamais journalisées : seulement leurs noms.
                let champs: Vec<String> = patch.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
                let vars: Vec<String> = patch["env"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
                self.orch.record("mcp_modifie", json!({ "id": id, "nom": server["name"], "champs": champs, "variables": vars, "origine": "administration" }));
                self.orch.refresh_connectors();
                self.mcp_state()
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    fn mcp_remove(&self, id: &str) -> J {
        let Some(removed) = self.orch.mcp_registry.remove(id) else { return json!({ "ok": false, "error": "connecteur introuvable" }) };
        self.orch.record("mcp_supprime", json!({ "id": id, "nom": removed["name"], "origine": "administration" }));
        self.orch.refresh_connectors();
        self.mcp_state()
    }

    /// Lance le serveur, liste ses outils (repris dans la section Sécurité), puis l'arrête.
    async fn mcp_test(&self, id: &str) -> J {
        let reg = &self.orch.mcp_registry;
        let Some(server) = reg.get(id) else { return json!({ "ok": false, "error": "connecteur introuvable" }) };
        let result = if js::gs(&server, "transport") == "http" {
            super::mcp_client::list_tools_http(&js::gs(&server, "url"), &HashMap::new(), 30000).await
        } else {
            super::mcp_client::list_tools_stdio(&js::gs(&server, "command"), &js::str_list(&server["args"]), &reg.env_of(&server), 150000).await
        };
        reg.set_test_result(id, &result);
        let ok = js::gb(&result, "ok");
        let tools = if ok { js::arr(&result["tools"]).len() } else { 0 };
        self.orch.record(
            "mcp_teste",
            json!({ "id": id, "nom": server["name"], "ok": ok, "outils": tools, "erreur": if ok { Value::Null } else { result["error"].clone() }, "duree_ms": result["ms"], "origine": "administration" }),
        );
        self.orch.refresh_connectors();
        spread(self.mcp_state(), json!({ "test": { "ok": ok, "error": if js::truthy(&result["error"]) { result["error"].clone() } else { Value::Null }, "tools": tools, "ms": result["ms"] } }))
    }

    /// Authentification du connecteur (Spotify, Google) : ouvre le navigateur de l'utilisateur.
    async fn mcp_auth(&self, id: &str) -> J {
        let reg = &self.orch.mcp_registry;
        let Some(server) = reg.get(id) else { return json!({ "ok": false, "error": "connecteur introuvable" }) };
        let Some(args) = reg.auth_args(&server) else { return json!({ "ok": false, "error": "ce connecteur ne demande pas d’authentification" }) };
        let missing = reg.missing_required(&server);
        if !missing.is_empty() {
            return json!({ "ok": false, "error": format!("à renseigner d'abord : {}", missing.join(", ")) });
        }
        let result = super::mcp_client::run_once(&js::gs(&server, "command"), &args, &reg.env_of(&server), 300000).await;
        self.orch.record("mcp_modifie", json!({ "id": id, "nom": server["name"], "champs": ["authentification"], "ok": result["ok"], "origine": "administration" }));
        let out = js::gs(&result, "output");
        let n = out.chars().count();
        let tail: String = out.chars().skip(n.saturating_sub(1500)).collect();
        spread(self.mcp_state(), json!({ "auth": { "ok": js::gb(&result, "ok"), "output": tail, "error": if js::truthy(&result["error"]) { result["error"].clone() } else { Value::Null } } }))
    }

    // --- Codes de confirmation -------------------------------------------------------------------

    /// Le code est renvoyé à la page pour être affiché : le retaper est un geste délibéré. Il n'est
    /// jamais journalisé.
    fn challenge(&self, action: &str, payload: J, label: &str) -> J {
        let now = js::now_ms();
        self.challenges.borrow_mut().retain(|_, c| c.expires_at >= now);
        let mut b = [0u8; 8];
        let _ = getrandom::getrandom(&mut b);
        let id = hex::encode(b);
        let code = (1000 + js::rand_below(9000)).to_string();
        self.challenges.borrow_mut().insert(
            id.clone(),
            Challenge { code: code.clone(), action: action.into(), payload, label: label.into(), expires_at: now + CHALLENGE_TTL },
        );
        self.orch.record("validation_demandee", json!({ "action": action, "detail": label, "origine": "administration", "type": "code_ecrit" }));
        json!({ "ok": true, "challenge": { "id": id, "code": code, "label": label, "expiresIn": CHALLENGE_TTL / 1000 } })
    }

    fn consume(&self, id: &str, code: &str, action: &str) -> Result<J, J> {
        let mut chs = self.challenges.borrow_mut();
        let Some(ch) = chs.get(id).filter(|c| c.action == action) else { return Err(json!({ "ok": false, "error": "demande de validation introuvable ou expirée" })) };
        if ch.expires_at < js::now_ms() {
            chs.remove(id);
            return Err(json!({ "ok": false, "error": "code expiré" }));
        }
        if code.trim() != ch.code {
            let label = ch.label.clone();
            drop(chs);
            self.orch.record("validation_refusee", json!({ "action": action, "detail": label, "motif": "code erroné", "origine": "administration" }));
            return Err(json!({ "ok": false, "error": "code incorrect" }));
        }
        let ch = chs.remove(id).unwrap();
        drop(chs);
        self.orch.record("validation_accordee", json!({ "action": action, "detail": ch.label, "origine": "administration", "type": "code_ecrit" }));
        Ok(ch.payload)
    }

    // --- Empreinte vocale & Locuteur -------------------------------------------------------------

    fn voiceprint(&self) -> (PathBuf, Option<J>) {
        let p = self.orch.voiceprint_path();
        let d = js::read_json(&p);
        (p, d)
    }

    fn write_voiceprint(p: &Path, data: &J) -> Result<(), String> {
        std::fs::write(p, serde_json::to_string_pretty(data).unwrap_or_default()).map_err(|e| e.to_string())
    }

    fn speaker_set(&self, patch: &J) -> J {
        let status = self.orch.get_speaker_status();
        let enabled = if patch.get("enabled").is_some() && !patch["enabled"].is_null() { js::truthy(&patch["enabled"]) } else { js::truthy(&status["enabled"]) };
        let threshold = match js::num(&patch["threshold"]) {
            Some(t) if t.is_finite() => t.clamp(0.40, 0.90),
            _ => js::gn(&status, "threshold").unwrap_or(0.65),
        };
        let (p, data) = self.voiceprint();
        let mut data = data.unwrap_or_else(|| json!({}));
        data["enabled"] = json!(enabled);
        data["threshold"] = json!(threshold);
        if let Err(e) = Self::write_voiceprint(&p, &data) {
            return json!({ "ok": false, "error": e });
        }
        self.orch.emit(json!({ "type": "speaker_set_enabled", "enabled": enabled }));
        self.orch.emit(json!({ "type": "speaker_set_threshold", "threshold": threshold }));
        self.orch.publish_status();
        self.orch.record("changement_locuteur", json!({ "enabled": enabled, "threshold": threshold, "origine": "administration" }));
        json!({ "ok": true, "speaker": self.orch.get_speaker_status() })
    }

    fn speaker_reset(&self) -> J {
        let p = self.orch.voiceprint_path();
        if p.exists() {
            if let Err(e) = std::fs::remove_file(&p) {
                return json!({ "ok": false, "error": e.to_string() });
            }
        }
        self.orch.emit(json!({ "type": "speaker_reset" }));
        self.orch.publish_status();
        self.orch.record("reinitialisation_locuteur", json!({ "origine": "administration" }));
        json!({ "ok": true, "speaker": self.orch.get_speaker_status() })
    }

    fn speaker_enroll(&self, a1: &J, a2: &J) -> J {
        let mut name = "Utilisateur".to_string();
        let mut count = 8i64; // 8 échantillons (casque puis haut-parleurs)
        let mut speaker_type: Option<String> = None;
        if a1.is_object() {
            name = Some(js::gs(a1, "name")).filter(|n| !n.is_empty()).unwrap_or(name);
            count = js::gn(a1, "count").filter(|c| *c != 0.0).map(|c| c as i64).unwrap_or(8);
            let t = js::gs(a1, "type");
            if SPEAKER_TYPES.contains(&t.as_str()) {
                speaker_type = Some(t);
            }
        } else {
            let n = js::s(a1);
            if !n.trim().is_empty() {
                name = n.trim().to_string();
            }
            if let Some(c) = js::num(a2).filter(|c| *c != 0.0) {
                count = c as i64;
            }
        }
        let mut ev = json!({ "type": "speaker_enroll_start", "name": name, "count": count });
        let mut rec = json!({ "nom": name, "echantillons": count });
        if let Some(t) = &speaker_type {
            ev["speakerType"] = json!(t);
            rec["lien"] = json!(t);
        }
        rec["origine"] = json!("administration");
        self.orch.emit(ev);
        self.orch.record("enrolement_locuteur_demarre", rec);
        json!({ "ok": true, "message": format!("Enrôlement démarré pour {name}") })
    }

    fn target_id(target: &J) -> String {
        let id = if target.is_object() { js::gs(target, "id") } else { js::s(target) };
        id.trim().to_lowercase()
    }

    fn speaker_delete(&self, target: &J) -> J {
        let id = Self::target_id(target);
        if id.is_empty() {
            return json!({ "ok": false, "error": "Identifiant locuteur requis" });
        }
        let (p, data) = self.voiceprint();
        if let Some(mut data) = data {
            if let Some(list) = data["speakers"].as_array().cloned() {
                let mut speakers: Vec<J> = list.into_iter().filter(|s| js::s(&s["id"]).trim().to_lowercase() != id).collect();
                data["speakersCount"] = json!(speakers.len());
                data["enrolled"] = json!(!speakers.is_empty());
                if !speakers.is_empty() {
                    if !speakers.iter().any(|s| js::truthy(&s["isPrimary"])) {
                        speakers[0]["isPrimary"] = json!(true);
                    }
                    let primary = speakers.iter().find(|s| js::truthy(&s["isPrimary"])).cloned().unwrap_or_else(|| speakers[0].clone());
                    data["embedding"] = primary["embedding"].clone();
                    data["enrolledAt"] = primary["enrolledAt"].clone();
                    data["samplesCount"] = primary["samplesCount"].clone();
                } else {
                    data["embedding"] = Value::Null;
                    data["enrolledAt"] = Value::Null;
                    data["samplesCount"] = json!(0);
                }
                data["speakers"] = json!(speakers);
                if let Err(e) = Self::write_voiceprint(&p, &data) {
                    return json!({ "ok": false, "error": e });
                }
            }
        }
        self.orch.emit(json!({ "type": "speaker_delete", "id": id }));
        self.orch.publish_status();
        self.orch.record("suppression_locuteur", json!({ "id": id, "origine": "administration" }));
        json!({ "ok": true, "speaker": self.orch.get_speaker_status() })
    }

    fn speaker_rename(&self, target: &J, name: &J) -> J {
        let id = Self::target_id(target);
        let new_name = if target.is_object() && js::truthy(&target["name"]) { js::gs(target, "name") } else { js::s(name) };
        let clean = new_name.trim().to_string();
        if id.is_empty() || clean.is_empty() {
            return json!({ "ok": false, "error": "Identifiant et nouveau nom requis" });
        }
        let (p, data) = self.voiceprint();
        if let Some(mut data) = data {
            let mut changed = false;
            if let Some(list) = data["speakers"].as_array_mut() {
                if let Some(s) = list.iter_mut().find(|s| js::s(&s["id"]).trim().to_lowercase() == id) {
                    s["name"] = json!(clean);
                    changed = true;
                }
            }
            if changed {
                if let Err(e) = Self::write_voiceprint(&p, &data) {
                    return json!({ "ok": false, "error": e });
                }
            }
        }
        self.orch.emit(json!({ "type": "speaker_rename", "id": id, "name": clean }));
        self.orch.publish_status();
        self.orch.record("renommage_locuteur", json!({ "id": id, "nom": clean, "origine": "administration" }));
        json!({ "ok": true, "speaker": self.orch.get_speaker_status() })
    }

    /// Lien d'une personne avec l'utilisateur principal (fille, femme, ami…).
    fn speaker_set_type(&self, target: &J, speaker_type: &str) -> J {
        let id = Self::target_id(target);
        if id.is_empty() || !SPEAKER_TYPES.contains(&speaker_type) {
            return json!({ "ok": false, "error": "Personne ou lien invalide" });
        }
        let (p, data) = self.voiceprint();
        if let Some(mut data) = data {
            let mut changed = false;
            if let Some(list) = data["speakers"].as_array_mut() {
                if let Some(s) = list.iter_mut().find(|s| js::s(&s["id"]).trim().to_lowercase() == id) {
                    s["type"] = json!(speaker_type);
                    changed = true;
                }
            }
            if changed {
                if let Err(e) = Self::write_voiceprint(&p, &data) {
                    return json!({ "ok": false, "error": e });
                }
            }
        }
        self.orch.emit(json!({ "type": "speaker_set_type", "id": id, "speakerType": speaker_type }));
        self.orch.publish_status();
        self.orch.record("changement_locuteur", json!({ "id": id, "lien": speaker_type, "origine": "administration" }));
        json!({ "ok": true, "speaker": self.orch.get_speaker_status() })
    }

    /// Filtrage strict (« reject ») ou souple (« signal »), marge de doute et apprentissage automatique.
    pub fn speaker_options(&self, patch: &J) -> J {
        let mut out = Map::new();
        let mode = js::gs(patch, "mode");
        if mode == "reject" || mode == "signal" {
            out.insert("mode".into(), json!(mode));
        }
        if let Some(m) = js::num(&patch["doubtMargin"]).filter(|m| m.is_finite()) {
            out.insert("doubtMargin".into(), json!(m.clamp(0.0, 0.3)));
        }
        if let Some(m) = js::num(&patch["nearMiss"]).filter(|m| m.is_finite()) {
            out.insert("nearMiss".into(), json!(m.clamp(0.0, 0.2)));
        }
        for k in ["autoLearn", "learning"] {
            if patch.get(k).is_some() && !patch[k].is_null() {
                out.insert(k.into(), json!(js::truthy(&patch[k])));
            }
        }
        let out = J::Object(out);
        let (p, data) = self.voiceprint();
        if let Some(mut data) = data {
            js::assign(&mut data, out.clone());
            if let Err(e) = Self::write_voiceprint(&p, &data) {
                return json!({ "ok": false, "error": e });
            }
        }
        if js::truthy(&out["mode"]) {
            self.orch.emit(json!({ "type": "voice_command", "payload": { "t": "speaker_set_mode", "mode": out["mode"] } }));
        }
        if out.get("doubtMargin").is_some() || out.get("autoLearn").is_some() || out.get("learning").is_some() {
            self.orch.emit(json!({ "type": "speaker_set_options", "doubtMargin": out["doubtMargin"], "autoLearn": out["autoLearn"], "learning": out["learning"] }));
        }
        self.orch.publish_status();
        let mut rec = out.clone();
        rec["origine"] = json!(if js::gs(patch, "origin") == "hud" { "HUD" } else { "administration" });
        self.orch.record("changement_mode_locuteur", rec);
        json!({ "ok": true, "speaker": self.orch.get_speaker_status() })
    }

    fn speaker_primary(&self, target: &J) -> J {
        let id = Self::target_id(target);
        if id.is_empty() {
            return json!({ "ok": false, "error": "Identifiant requis" });
        }
        let (p, data) = self.voiceprint();
        if let Some(mut data) = data {
            if let Some(list) = data["speakers"].as_array_mut() {
                for s in list.iter_mut() {
                    s["isPrimary"] = json!(js::s(&s["id"]).trim().to_lowercase() == id);
                }
                let primary = list.iter().find(|s| js::truthy(&s["isPrimary"])).cloned();
                if let Some(pr) = primary {
                    data["embedding"] = pr["embedding"].clone();
                    data["enrolledAt"] = pr["enrolledAt"].clone();
                    data["samplesCount"] = pr["samplesCount"].clone();
                }
                if let Err(e) = Self::write_voiceprint(&p, &data) {
                    return json!({ "ok": false, "error": e });
                }
            }
        }
        self.orch.emit(json!({ "type": "speaker_primary", "id": id }));
        self.orch.publish_status();
        self.orch.record("changement_locuteur_principal", json!({ "id": id, "origine": "administration" }));
        json!({ "ok": true, "speaker": self.orch.get_speaker_status() })
    }
}
