// API des modules de Jarvis (modules/<id>/core) : ce qu'un module reçoit du noyau pour exister.
//
// Un module est une card (ou un service) indépendant : une fiche (module.json), une interface (ui/) et, s'il a
// une logique propre, une crate Rust qui implémente `Module`. Le noyau ne connaît pas les modules : le binaire
// jarvis-core.exe (crates/jarvis-app) lui passe la liste fournie par crates/jarvis-modules.
//
// Ce qu'un module peut faire passe par `ModuleCtx` : déclarer des méthodes appelables par les fenêtres,
// diffuser des événements, écrire dans le journal, ranger ses fichiers dans data/modules/<id>/, et demander au
// cœur les rares services partagés (`CoreAccess`). On n'y ajoute que ce dont un module a réellement besoin.

use std::path::PathBuf;
use std::rc::{Rc, Weak};

use crate::gateway::{Client, Gateway};
use crate::js::J;
use crate::orch::Fut;

/// Partie noyau d'un module.
pub trait Module {
    /// Identifiant : le nom du dossier modules/<id>/ et le champ « id » de module.json.
    fn id(&self) -> &'static str;

    /// Sous-commande de jarvis-core.exe propre au module (processus auxiliaire), avant tout démarrage du
    /// noyau ; `Some(code)` = traitée. Appelée même si le module est désactivé (le processus est autonome).
    fn subcommand(&self, _cmd: &str, _args: &[String]) -> Option<i32> {
        None
    }

    /// Démarrage, une fois la passerelle prête : le module déclare ses méthodes et lance ce qu'il faut.
    fn start(&self, ctx: Rc<ModuleCtx>);

    /// Arrêt du noyau : processus auxiliaires fermés, minuteries coupées.
    fn stop(&self) {}
}

/// Services du cœur accessibles aux modules (implémentés par app.rs).
pub trait CoreAccess {
    /// Événement de l'orchestrateur (voix, coquille, puis fenêtres), comme s'il venait du cœur.
    fn emit(&self, ev: J);
    /// Ligne du journal scellé (type, données).
    fn record(&self, kind: &str, data: J);
    /// Volume propre de la voix de Jarvis (0 à 100).
    fn jarvis_volume(&self) -> i64;
    /// Sortie ou micro par défaut changé : le moteur vocal rouvre ses flux, les fenêtres sont prévenues.
    fn audio_device_changed(&self) -> Fut<()>;
}

/// Contexte d'un module démarré.
pub struct ModuleCtx {
    pub id: String,
    gateway: Rc<Gateway>,
    core: Weak<dyn CoreAccess>,
    log_fn: Rc<dyn Fn(String)>,
    data_dir: PathBuf,
}

impl ModuleCtx {
    pub fn new(id: &str, gateway: Rc<Gateway>, core: Weak<dyn CoreAccess>, log_fn: Rc<dyn Fn(String)>, data_dir: PathBuf) -> Rc<Self> {
        Rc::new(Self { id: id.to_string(), gateway, core, log_fn, data_dir })
    }

    /// Méthode appelable par les fenêtres des rôles donnés (« hud », « widget », « admin »). Un nom déjà pris
    /// par le cœur ou un autre module est refusé (journal) : un module ne remplace jamais une méthode existante.
    pub fn register<F>(&self, name: &str, roles: &[&str], f: F)
    where
        F: Fn(Vec<J>, Rc<Client>) -> Fut<Result<J, String>> + 'static,
    {
        if self.gateway.has_method(name) {
            self.log(&format!("méthode « {name} » déjà déclarée : ignorée"));
            return;
        }
        self.gateway.register(name, roles, f);
    }

    /// Événement vers les fenêtres (toutes, ou celles des rôles donnés).
    pub fn broadcast(&self, ev: &J, roles: Option<&[&str]>) {
        self.gateway.broadcast(ev, roles);
    }

    pub fn log(&self, msg: &str) {
        (self.log_fn)(format!("[module {}] {msg}", self.id));
    }

    /// Dossier de données du module (data/modules/<id>/), créé au besoin.
    pub fn data_dir(&self) -> PathBuf {
        let _ = std::fs::create_dir_all(&self.data_dir);
        self.data_dir.clone()
    }

    /// Services du cœur ; `None` pendant l'arrêt.
    pub fn core(&self) -> Option<Rc<dyn CoreAccess>> {
        self.core.upgrade()
    }
}
