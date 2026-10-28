// Interface servie depuis le disque (ui/) au lieu des fichiers figés dans l'exécutable à la compilation.
//
// Une modification de ui/ se voit alors par un simple rechargement des fenêtres, sans recompiler la
// coquille (la compilation prend plusieurs minutes). Les fichiers intégrés restent le repli : exécutable
// déplacé hors du projet, fichier absent, JARVIS_UI_EMBARQUEE=1 pour les imposer.

use std::borrow::Cow;
use std::path::PathBuf;

use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};
use tauri::{Assets, Runtime};

pub struct DiskAssets<R: Runtime> {
    dir: PathBuf,
    embedded: Box<dyn Assets<R>>,
}

impl<R: Runtime> DiskAssets<R> {
    /// `None` si le dossier ui/ n'est pas présent à côté de l'exécutable, ou si l'on demande l'intégré.
    pub fn new(root: &std::path::Path, embedded: Box<dyn Assets<R>>) -> Result<Self, Box<dyn Assets<R>>> {
        let dir = root.join("ui");
        if std::env::var("JARVIS_UI_EMBARQUEE").is_ok() || !dir.join("index.html").is_file() {
            return Err(embedded);
        }
        Ok(Self { dir, embedded })
    }
}

impl<R: Runtime> Assets<R> for DiskAssets<R> {
    fn setup(&self, app: &tauri::App<R>) {
        self.embedded.setup(app);
    }

    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        let rel = key.as_ref().trim_start_matches('/');
        // Jamais de remontée de dossier : seuls les fichiers de ui/ et l'interface des modules (modules/<id>/ui/,
        // à côté de ui/) sont servis.
        if !rel.is_empty() && !rel.split('/').any(|p| p == ".." || p.contains(':') || p.contains('\\')) {
            let parts: Vec<&str> = rel.split('/').collect();
            let file = match (parts.first(), parts.get(2), self.dir.parent()) {
                // Modules créés par l'utilisateur (data/user-modules/perso-<id>/ui/…), hors du dépôt.
                (Some(&"modules"), Some(&"ui"), Some(root)) if parts.len() > 3 && parts[1].starts_with("perso-") => {
                    let data = crate::data_dir(root);
                    data.join("user-modules").join(parts[1..].join("/"))
                }
                (Some(&"modules"), Some(&"ui"), Some(root)) if parts.len() > 3 => root.join(rel),
                _ => self.dir.join(rel),
            };
            if let Ok(bytes) = std::fs::read(file) {
                return Some(Cow::Owned(bytes));
            }
        }
        self.embedded.get(key)
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        self.embedded.iter()
    }

    fn csp_hashes(&self, html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        self.embedded.csp_hashes(html_path)
    }
}
