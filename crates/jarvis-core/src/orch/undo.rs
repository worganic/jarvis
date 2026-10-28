// « Annule ce que tu viens de faire » / « défais » / « reviens en arrière » (orchestrator/undo.js) :
// Jarvis défait sa dernière action directe. Chaque action réversible laisse une entrée dans une pile
// (20 au plus, 2 h au plus) avec son geste inverse, qui repasse par local_tool (même sécurité).

use std::cell::{Cell, RefCell};

use serde_json::{json, Value};

use super::{system, Fut, OrchRef, Up};
use crate::js::{self, J};

const MAX_ITEMS: usize = 20;
const MAX_AGE_MS: i64 = 2 * 3_600_000;

pub type UndoFn = Box<dyn FnOnce() -> Fut<String>>;

struct Item {
    label: String,
    undo: UndoFn,
    at: i64,
}

pub struct UndoStack {
    o: OrchRef,
    items: RefCell<Vec<Item>>,
    replaying: Cell<bool>,
}

/// Outil système rejoué par local_tool (même politique de sécurité).
fn tool2(o: &OrchRef, name: &'static str, p: J) -> Fut<J> {
    let o = o.up();
    Box::pin(async move {
        o.local_tool(name, p, Box::new(move |q| Box::pin(async move { system::call(name.trim_start_matches("mcp__system__"), &q).await.unwrap_or(Ok(Value::Null)).unwrap_or_else(|e| json!({ "ok": false, "error": e })) })))
            .await
    })
}

impl UndoStack {
    pub fn new(o: OrchRef) -> Self {
        Self { o, items: RefCell::new(vec![]), replaying: Cell::new(false) }
    }

    fn push(&self, label: String, undo: UndoFn) {
        let mut items = self.items.borrow_mut();
        items.push(Item { label, undo, at: js::now_ms() });
        if items.len() > MAX_ITEMS {
            items.remove(0);
        }
    }

    /// Avant une action : ce qu'il faut relever pour pouvoir la défaire (luminosité, presse-papier, lumières).
    pub async fn prepare(&self, tool: &str, params: &J) -> Option<J> {
        match tool {
            "mcp__system__luminosite" => Some(json!({ "brightness": system::luminosite_actuelle().await })),
            "mcp__system__lumiere" if js::gs(params, "action") != "lister" => {
                let lights = self.o.up().lights.snapshot(&js::gs(params, "lampe")).await;
                Some(json!({ "lights": lights }))
            }
            "mcp__system__presse_papier_ecrire" => {
                let r = system::presse_papier_lire(&json!({})).await.ok();
                Some(json!({ "clipboard": r.map(|r| js::gs(&r, "texte")).unwrap_or_default() }))
            }
            _ => None,
        }
    }

    /// Après une action réussie : son inverse est empilé (si elle en a un).
    pub fn record(&self, tool: &str, params: &J, result: &J, before: Option<J>) {
        if self.replaying.get() {
            return;
        }
        if !js::truthy(result) || result["ok"] == json!(false) || js::truthy(&result["error"]) || js::truthy(&result["denied"]) {
            return;
        }
        let o = self.o.clone();
        match tool {
            "mcp__system__volume" => {
                let a = js::gs(params, "action");
                if !["up", "down", "mute"].contains(&a.as_str()) {
                    return;
                }
                let back = match a.as_str() {
                    "up" => "down",
                    "down" => "up",
                    _ => "mute",
                };
                let label = if a == "mute" { "coupure du son".to_string() } else { format!("son {}", if a == "up" { "monté" } else { "baissé" }) };
                self.push(
                    label,
                    Box::new(move || {
                        Box::pin(async move {
                            tool2(&o, "mcp__system__volume", json!({ "action": back })).await;
                            if a == "mute" { "Son rétabli.".to_string() } else { format!("Son {} comme avant.", if a == "up" { "baissé" } else { "remonté" }) }
                        })
                    }),
                );
            }
            "mcp__system__luminosite" => {
                let Some(prev) = before.and_then(|b| js::num(&b["brightness"]).filter(|_| !b["brightness"].is_null())) else { return };
                self.push(
                    "luminosité".into(),
                    Box::new(move || {
                        Box::pin(async move {
                            tool2(&o, "mcp__system__luminosite", json!({ "pourcentage": prev })).await;
                            format!("Luminosité remise à {} %.", js::num_str(prev))
                        })
                    }),
                );
            }
            "mcp__system__fermer_application" => {
                let nom = js::gs(params, "nom");
                if nom.is_empty() {
                    return;
                }
                self.push(
                    format!("fermeture de {nom}"),
                    Box::new(move || {
                        Box::pin(async move {
                            tool2(&o, "mcp__system__ouvrir_application", json!({ "nom": nom })).await;
                            format!("Je rouvre {nom}.")
                        })
                    }),
                );
            }
            "mcp__system__ouvrir_application" => {
                let nom = js::gs(params, "nom");
                if nom.is_empty() {
                    return;
                }
                self.push(
                    format!("ouverture de {nom}"),
                    Box::new(move || {
                        Box::pin(async move {
                            let r = tool2(&o, "mcp__system__fermer_application", json!({ "nom": nom })).await;
                            if js::gb(&r, "denied") { "Fermeture refusée.".to_string() } else { format!("Je ferme {nom}.") }
                        })
                    }),
                );
            }
            "mcp__system__presse_papier_ecrire" => {
                let Some(prev) = before.and_then(|b| b["clipboard"].as_str().map(String::from)) else { return };
                self.push(
                    "presse-papier".into(),
                    Box::new(move || {
                        Box::pin(async move {
                            tool2(&o, "mcp__system__presse_papier_ecrire", json!({ "texte": prev })).await;
                            "Presse-papier remis comme avant.".to_string()
                        })
                    }),
                );
            }
            "mcp__system__lumiere" => {
                let Some(states) = before.map(|b| b["lights"].clone()).filter(|l| l.as_array().map(|a| !a.is_empty()).unwrap_or(false)) else { return };
                self.push(
                    "lumières".into(),
                    Box::new(move || {
                        Box::pin(async move {
                            o.up().lights.restore(&states).await;
                            "Lumières remises comme avant.".to_string()
                        })
                    }),
                );
            }
            "mcp__google__agenda_creer" => {
                let ev = result["evenement"].clone();
                let id = js::gs(&ev, "id");
                if id.is_empty() {
                    return;
                }
                let titre = js::gs(&ev, "titre");
                self.push(
                    format!("rendez-vous « {titre} »"),
                    Box::new(move || {
                        Box::pin(async move {
                            let r = o.up().google.call("agenda_supprimer", &json!({ "id": id })).await.unwrap_or_else(|e| json!({ "ok": false, "error": e }));
                            if js::gb(&r, "ok") { format!("Rendez-vous « {titre} » retiré.") } else { "Je n’ai pas pu retirer ce rendez-vous.".to_string() }
                        })
                    }),
                );
            }
            _ => {}
        }
    }

    /// Entrées ajoutées hors outils (rappel créé, raccourci créé…).
    pub fn add(&self, label: &str, undo: UndoFn) {
        self.push(label.to_string(), undo);
    }

    pub async fn undo_last(&self) -> J {
        let last = {
            let mut items = self.items.borrow_mut();
            while items.last().map(|l| js::now_ms() - l.at > MAX_AGE_MS).unwrap_or(false) {
                items.pop();
            }
            items.pop()
        };
        let Some(last) = last else {
            return json!({ "say": "Je n'ai rien à annuler. Je peux défaire le volume, la luminosité, une application ouverte ou fermée, le presse-papier, un rappel, un rendez-vous ou un raccourci que je viens de créer.", "display": "Rien à annuler" });
        };
        self.replaying.set(true);
        let label = last.label.clone();
        let say = (last.undo)().await;
        self.replaying.set(false);
        self.o.up().record("action_annulee", json!({ "action": label }));
        json!({ "say": if say.is_empty() { format!("J'ai annulé : {label}.") } else { say }, "display": format!("Annulé : {label}") })
    }
}

/// « Annule » seul reste l'annulation d'une tâche : il faut préciser.
pub fn is_undo(folded: &str) -> bool {
    crate::re!(r"^(?:annule (?:ce que tu viens de faire|ta derniere action|la derniere action|ce que tu as fait|l action precedente)|(?:defais|defait|reviens en arriere|retour en arriere|retour arriere|ctrl z|controle z)(?: (?:ce que tu viens de faire|ta derniere action|la derniere action|ce que tu as fait|ca|cela))?)(?: stp| s il te plait)?$").is_match(folded)
}
