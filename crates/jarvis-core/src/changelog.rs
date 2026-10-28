// Version courante de Jarvis : première entrée de ui/changelog.json (source unique de l'historique,
// affichée par le HUD, l'écran de mise à jour et la page ui/changelog.html). Lue à la demande : la
// version d'une mise à jour qui vient d'être écrite est visible sans recompiler.

use serde_json::json;

use crate::js::{self, J};

pub struct Version {
    pub version: String,
    pub date: String,
    pub title: String,
}

/// Version la plus récente (les entrées sont classées du plus récent au plus ancien).
pub fn current() -> Option<Version> {
    current_in(&crate::paths::root())
}

/// Idem pour une racine donnée (l'écran de mise à jour tourne depuis une copie dans le dossier temporaire).
pub fn current_in(root: &std::path::Path) -> Option<Version> {
    let file = root.join("ui").join("changelog.json");
    let doc = js::read_json(&file)?;
    let first = doc["versions"].as_array()?.first()?;
    let version = js::gs(first, "version");
    (!version.is_empty()).then(|| Version { version, date: js::gs(first, "date"), title: js::gs(first, "title") })
}

/// « 4.88.0 » → [4, 88, 0] (comparaison des versions).
fn parts(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

/// Entrées de l'historique plus récentes que `than` (la version qui tournait avant la mise à jour), de la
/// plus récente à la plus ancienne : { version, title, changes: [texte] }.
pub fn newer_in(root: &std::path::Path, than: &str) -> Vec<J> {
    let Some(doc) = js::read_json(&root.join("ui").join("changelog.json")) else { return vec![] };
    js::arr(&doc["versions"])
        .into_iter()
        .take_while(|v| parts(&js::gs(v, "version")) > parts(than))
        .map(|v| json!({ "version": v["version"], "title": v["title"], "changes": js::arr(&v["changes"]).iter().map(|c| js::gs(c, "text")).collect::<Vec<_>>() }))
        .collect()
}

/// Réponse de `appVersion` : { version, date, title } (version vide si l'historique est illisible).
pub fn info() -> J {
    match current() {
        Some(v) => json!({ "version": v.version, "date": v.date, "title": v.title }),
        None => json!({ "version": "", "date": "", "title": "" }),
    }
}
