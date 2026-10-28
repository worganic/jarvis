// Données vivantes de la page d'aide (ui/aide.html, ouverte par le bouton « ? AIDE » du HUD).
//
// La page contient le texte rédigé (ui/aide-contenu.json : fonctions, fonctionnement, raccourcis) et
// complète avec ce qui change tout seul, servi par la passerelle à /aide/donnees.json :
//   - les commandes du catalogue (aide aux commandes, compétences apprises comprises) ;
//   - les capacités de Jarvis et leur niveau d'accord actuel (Sécurité) ;
//   - toutes les informations des RAG : mémoire long terme, routeur sémantique, aide aux commandes,
//     cache des réponses, réunions ;
//   - l'état courant : moteur IA, voix, connecteurs, zones d'accès.
// Jamais le texte des souvenirs, des mails ou des réunions : seulement des chiffres et des réglages.

use serde_json::json;

use super::{answer_cache, embeddings, memory, semantic_router, Orchestrator};
use crate::js::{self, J};

impl Orchestrator {
    pub fn help_data(&self) -> J {
        let emb = self.embedder.as_ref();
        let model = emb.map(|e| e.model.clone()).unwrap_or_default();
        let installed = emb.map(|e| e.cache_dir.join(e.model.replace('/', std::path::MAIN_SEPARATOR_STR)).join("onnx").exists()).unwrap_or(false);
        let loaded = emb.map(|e| e.active()).unwrap_or(false);

        let commands: Vec<J> = self
            .command_help
            .all()
            .iter()
            .map(|c| json!({ "categorie": c["cat"], "dire": c["say"], "effet": c["what"], "mots": c["keys"] }))
            .collect();
        let categories: Vec<String> = commands.iter().fold(vec![], |mut v, c| {
            let n = js::gs(c, "categorie");
            if !v.contains(&n) {
                v.push(n);
            }
            v
        });

        let phrases: usize = semantic_router::EXAMPLES.iter().map(|(_, ex)| ex.len() + 1).sum();
        let mut memoire = self.memory.stats();
        js::assign(
            &mut memoire,
            json!({
                "modele": model, "modeleInstalle": installed, "modeleCharge": loaded, "dechargeApresMinutes": 5,
                "poidsSemantique": 0.75, "poidsLexical": 0.25,
                "seuilMinimal": memory::SEMANTIC_MIN, "seuilRelatif": memory::SEMANTIC_RELATIVE,
                "injectionMaxCaracteres": memory::MAX_PROMPT_CHARS, "injectionCategories": memory::PROMPT_CATEGORIES,
                "automatique": {
                    "active": self.config.get("memoryAuto")["enabled"] == json!(true),
                    "minutes": js::num(&self.config.get("memoryAuto")["idleMinutes"]).unwrap_or(5.0),
                    "enAttente": self.memory_extract.pending(), "analyses": self.memory_extract.journal().len()
                }
            }),
        );

        let status = self.status.d.borrow();
        let state = json!({
            "moteur": status.engine, "modele": status.model, "modeleActif": status.active_model, "effort": status.effort,
            "economie": status.economy_level, "voix": status.voice_name, "debit": status.voice_rate,
            "modeEcoute": status.listening_mode, "theme": status.theme, "densite": status.density, "personnage": status.heart_style
        });
        drop(status);
        let connectors: Vec<String> = self
            .mcp_registry
            .enabled()
            .iter()
            .map(|s| Some(js::gs(s, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| js::gs(s, "id")))
            .collect();
        let zones: Vec<J> = self.zones.list().iter().map(|z| json!({ "nom": z["name"], "lecture": z["read"], "ecriture": z["write"] })).collect();
        let actions: Vec<J> = self
            .security
            .list()
            .iter()
            .map(|a| json!({ "groupe": a["group"], "libelle": a["label"], "description": a["description"], "risque": a["risk"], "niveauParDefaut": a["default"], "niveau": a["level"] }))
            .collect();

        json!({
            "genereLe": js::iso_now(),
            "version": crate::changelog::info(),
            "commandes": { "total": commands.len(), "categories": categories, "liste": commands },
            "actions": actions,
            "presence": self.presence.state(),
            "annoncesEnAttente": self.announcer.pending(),
            "rag": {
                "memoire": memoire,
                "routeur": {
                    "intentions": semantic_router::EXAMPLES.len(), "phrasesExemples": phrases, "seuil": semantic_router::THRESHOLD,
                    "liste": semantic_router::EXAMPLES.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
                    "modele": model
                },
                "aideCommandes": { "commandes": commands.len(), "categories": categories.len() },
                "cacheReponses": { "entrees": self.answer_cache.len(), "max": answer_cache::MAX, "dureeMinutes": answer_cache::TTL_MS / 60000 },
                "reunions": self.meetings.rag_stats(),
                "listes": self.collections.stats(),
                "documents": self.documents.stats(),
                "savoir": self.savoir.stats(),
                "modeleEmbeddings": { "nom": model, "installe": installed, "charge": loaded, "defaut": embeddings::DEFAULT_MODEL }
            },
            "etat": { "ia": state, "connecteurs": connectors, "zones": zones }
        })
    }
}

#[cfg(test)]
mod tests {
    /// « major.minor » d'un numéro de version.
    fn minor(v: &str) -> String {
        v.split('.').take(2).collect::<Vec<_>>().join(".")
    }

    /// Une nouvelle version mineure (= nouvelle fonction) doit s'accompagner d'une révision de la page
    /// d'aide : mettre à jour ui/aide-contenu.json puis y reporter la version dans « aJourVersion ».
    #[test]
    fn la_page_d_aide_suit_les_nouvelles_fonctions() {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("ui").join("aide-contenu.json");
        let text = std::fs::read_to_string(&file).expect("ui/aide-contenu.json introuvable");
        let doc: serde_json::Value = serde_json::from_str(&text).expect("ui/aide-contenu.json n'est pas du JSON valide");
        let revised = doc["aJourVersion"].as_str().expect("aJourVersion manquant dans ui/aide-contenu.json");
        assert_eq!(
            minor(revised),
            minor(env!("CARGO_PKG_VERSION")),
            "Jarvis est en v{} mais la page d'aide a été révisée pour la v{revised} : relis ui/aide-contenu.json (nouvelle fonction ? nouveau raccourci ?) puis mets à jour « aJourVersion ».",
            env!("CARGO_PKG_VERSION")
        );
        assert!(doc["sections"].as_array().map(|s| s.len() >= 8).unwrap_or(false), "la page d'aide a perdu des sections");
    }
}
