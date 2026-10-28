// Serveur MCP « jarvis » (orchestrator/mcp-server.js) : approbation des outils, mémoire, journal,
// précisions demandées à l'utilisateur, réunions. Tout est relayé à l'orchestrateur.

use std::rc::Rc;

use serde_json::json;

use super::{post, serve, text, text_s, J};
use crate::orch::memory::Memory;

fn tools() -> J {
    json!([
        {
            "name": "approve",
            "description": "Demande à Jarvis l'autorisation d'utiliser un outil.",
            "inputSchema": { "type": "object", "properties": { "tool_name": { "type": "string" }, "input": { "type": "object" }, "tool_use_id": { "type": "string" } }, "required": ["tool_name", "input"] }
        },
        {
            "name": "memoire_lire",
            "description": "Cherche dans la mémoire long terme de l'utilisateur (préférences, informations personnelles, habitudes). À utiliser dès qu'une demande dépend de ses goûts ou de son contexte.",
            "inputSchema": { "type": "object", "properties": { "recherche": { "type": "string", "description": "mots-clés ; vide = tout" } } }
        },
        {
            "name": "memoire_ajouter",
            "description": "Enregistre une information personnelle sur l'utilisateur (préférence, identité, habitude, projet), durable ou valable jusqu'à une date (expire_le) : elle est alors oubliée d'elle-même. Ne jamais y mettre de secret ni de mot de passe.",
            "inputSchema": { "type": "object", "properties": {
                "texte": { "type": "string", "description": "le fait à retenir, formulé en une phrase" },
                "categorie": { "type": "string", "description": "identité, préférences, habitudes, projets, contacts, divers" },
                "expire_le": { "type": "string", "description": "AAAA-MM-JJ : dernier jour où l'information reste vraie, pour une situation passagère (congés, déplacement, invité, traitement de quelques jours). Omettre pour une information durable." }
            }, "required": ["texte"] }
        },
        {
            "name": "historique_lire",
            "description": "Consulte le journal inviolable de Jarvis : tout ce qu'il a fait, demandé ou refusé. Lecture seule, aucune entrée ne peut être modifiée ni supprimée.",
            "inputSchema": { "type": "object", "properties": {
                "recherche": { "type": "string", "description": "mots-clés ; vide = entrées les plus récentes" },
                "limite": { "type": "number", "description": "nombre d'entrées, 20 par défaut" }
            } }
        },
        {
            "name": "memoire_oublier",
            "description": "Supprime une information de la mémoire. Action sensible : Jarvis demandera un code à l'utilisateur.",
            "inputSchema": { "type": "object", "properties": { "reference": { "type": "string", "description": "identifiant ou texte exact de l'entrée" } }, "required": ["reference"] }
        },
        {
            "name": "demander_precision",
            "description": "Pose une question à l'utilisateur pour lever une ambiguïté, demander un choix ou préciser un paramètre manquant. L'orchestrateur suspendra l'exécution, posera la question à voix haute et attendra la réponse de l'utilisateur avant de te la transmettre.",
            "inputSchema": { "type": "object", "properties": { "question": { "type": "string", "description": "la question claire, directe et concise à poser à voix haute à l'utilisateur" } }, "required": ["question"] }
        },
        {
            "name": "reunions_chercher",
            "description": "Cherche par le sens dans les réunions enregistrées par Jarvis (transcriptions et résumés, avec qui parlait). À utiliser pour toute question sur ce qui a été dit, décidé ou prévu en réunion, ou sur qui y participait. Chaque résultat donne le titre, la date, les participants et un extrait.",
            "inputSchema": { "type": "object", "properties": {
                "recherche": { "type": "string", "description": "le sujet cherché (ex. « budget 2027 », « ce que Marie a dit sur le devis »)" },
                "limite": { "type": "number", "description": "nombre de passages, 6 par défaut (12 au plus)" }
            }, "required": ["recherche"] }
        },
        {
            "name": "reunion_lire",
            "description": "Texte complet d'une réunion enregistrée (en-tête, résumé s'il existe, transcription horodatée avec les noms). Référence : « derniere », le numéro dans la liste (1 = la plus récente), le nom de fichier donné par reunions_chercher, ou des mots du titre ou d'un participant.",
            "inputSchema": { "type": "object", "properties": { "reference": { "type": "string", "description": "derniere, numéro, fichier ou mots" } }, "required": ["reference"] }
        },
        {
            "name": "listes_chercher",
            "description": "Cherche dans les listes à fiches de l'utilisateur (vinyles, Lego, livres… : ce qu'il possède ou voudrait posséder, avec prix, lieu et date d'achat). À utiliser pour « est-ce que j'ai… », « combien… », « quand ai-je acheté… ». Un numéro (set Lego) ou un titre est retrouvé à l'identique, le reste par le sens. Lecture seule.",
            "inputSchema": { "type": "object", "properties": {
                "recherche": { "type": "string", "description": "ce qui est cherché (ex. « Lego 75436 », « disques de U2 »)" },
                "liste": { "type": "string", "description": "nom de la liste pour restreindre (facultatif)" },
                "limite": { "type": "number", "description": "nombre de fiches, 8 par défaut (20 au plus)" }
            }, "required": ["recherche"] }
        },
        {
            "name": "listes_lire",
            "description": "Lit les listes à fiches de l'utilisateur. Sans argument : la liste de ses listes. Avec une liste : toutes ses fiches (titres, statut) et le total dépensé. Avec une liste et un élément (titre ou numéro) : la fiche complète. Lecture seule.",
            "inputSchema": { "type": "object", "properties": {
                "liste": { "type": "string", "description": "nom ou alias de la liste (ex. vinyles, lego)" },
                "element": { "type": "string", "description": "titre ou numéro de la fiche (facultatif)" }
            } }
        },
        {
            "name": "documents_chercher",
            "description": "Cherche par le sens dans les documents locaux que l'utilisateur a choisi de partager (dossiers suivis : PDF, Word, texte, Markdown, CSV). À utiliser pour « que dit mon contrat… », « retrouve dans mes documents… ». Chaque résultat donne le fichier, son chemin et un extrait. Lecture seule.",
            "inputSchema": { "type": "object", "properties": {
                "recherche": { "type": "string", "description": "le sujet ou la question (ex. « durée du préavis dans mon bail »)" },
                "limite": { "type": "number", "description": "nombre de passages, 6 par défaut (12 au plus)" }
            }, "required": ["recherche"] }
        },
        {
            "name": "savoir_chercher",
            "description": "Cherche par le sens dans les dossiers Data de l'utilisateur : les sujets qu'il a fait documenter par Jarvis (un appareil, un produit, un thème : fiche technique, histoire, fonctionnement, comparaisons, sources). À utiliser pour toute question sur l'un de ces sujets. Chaque résultat donne le dossier et un extrait. Lecture seule.",
            "inputSchema": { "type": "object", "properties": {
                "recherche": { "type": "string", "description": "la question ou le sujet (ex. « vitesse de croisière du H160 »)" },
                "dossier": { "type": "string", "description": "nom du dossier pour restreindre (facultatif, ex. « H160 »)" },
                "limite": { "type": "number", "description": "nombre de passages, 6 par défaut (12 au plus)" }
            }, "required": ["recherche"] }
        },
        {
            "name": "memoire_effacer",
            "description": "Efface et réinitialise intégralement la mémoire locale RAG de Jarvis. Tous les souvenirs et vecteurs sont purgés pour repartir avec une mémoire vierge. Action très sensible.",
            "inputSchema": { "type": "object", "properties": { "confirmation": { "type": "boolean", "description": "Doit être true pour confirmer l’effacement" } }, "required": ["confirmation"] }
        }
    ])
}

fn s(v: &J) -> String {
    match v {
        J::String(x) => x.clone(),
        J::Null => String::new(),
        other => other.to_string(),
    }
}

async fn call(name: String, args: J) -> J {
    let input = if args["input"].is_object() { args["input"].clone() } else { json!({}) };
    match name.as_str() {
        "approve" => {
            let v = post("/approve", json!({ "tool": args["tool_name"], "input": input })).await;
            if v["behavior"] == "allow" {
                text(json!({ "behavior": "allow", "updatedInput": input }))
            } else {
                text(json!({ "behavior": "deny", "message": if v["message"].is_string() { v["message"].clone() } else { json!("refusé par Jarvis") } }))
            }
        }
        "demander_precision" => {
            let r = post("/clarify", json!({ "question": args["question"] })).await;
            if r.get("error").map(|e| !e.is_null()).unwrap_or(false) {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            if r["answer"].as_str().map(|a| !a.is_empty()).unwrap_or(false) {
                text_s(format!("Réponse de l'utilisateur : \"{}\"", s(&r["answer"])))
            } else {
                text_s("Aucune réponse reçue de l’utilisateur.")
            }
        }
        "memoire_lire" => {
            let r = post("/memory", json!({ "op": "search", "query": s(&args["recherche"]) })).await;
            let entries = r["entries"].as_array().cloned().unwrap_or_default();
            if entries.is_empty() {
                return text_s("Aucune information mémorisée sur ce sujet.");
            }
            text_s(entries.iter().map(|e| format!("- [{}] {} (id {})", s(&e["category"]), Memory::text_with_expiry(e), s(&e["id"]))).collect::<Vec<_>>().join("\n"))
        }
        "memoire_ajouter" => {
            let r = post("/memory", json!({ "op": "add", "text": args["texte"], "category": args["categorie"], "expires": args["expire_le"] })).await;
            if !r["error"].is_null() {
                text_s(format!("Échec : {}", s(&r["error"])))
            } else {
                text_s(format!("Mémorisé : {}", Memory::text_with_expiry(&r["entry"])))
            }
        }
        "historique_lire" => {
            let r = post("/history", json!({ "query": s(&args["recherche"]), "limit": args["limite"] })).await;
            let entries = r["entries"].as_array().cloned().unwrap_or_default();
            if entries.is_empty() {
                return text_s("Journal vide pour cette recherche.");
            }
            let lines: Vec<String> = entries.iter().map(|e| format!("{} · {} · {}", s(&e["at"]), s(&e["type"]), e["data"])).collect();
            let seal = if r["integrity"]["ok"] == true {
                format!("Journal intact ({} entrées).", s(&r["integrity"]["count"]))
            } else {
                format!("ATTENTION : journal altéré ({}).", s(&r["integrity"]["reason"]))
            };
            text_s(format!("{}\n\n{seal}", lines.join("\n")))
        }
        "memoire_oublier" => {
            let r = post("/memory", json!({ "op": "forget", "reference": args["reference"] })).await;
            if !r["error"].is_null() {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            if r["entry"].is_object() {
                text_s(format!("Oublié : {}", s(&r["entry"]["text"])))
            } else {
                text_s("Aucune entrée correspondante.")
            }
        }
        "reunions_chercher" => {
            let r = post("/meetings", json!({ "op": "search", "query": s(&args["recherche"]), "limit": args["limite"] })).await;
            if !r["error"].is_null() {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            let results = r["results"].as_array().cloned().unwrap_or_default();
            if results.is_empty() {
                return text_s("Aucune réunion enregistrée ne correspond.");
            }
            text_s(
                results
                    .iter()
                    .map(|x| {
                        let parts: Vec<String> = x["participants"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default();
                        format!("### {} — {} (fichier {}{})\n{}", s(&x["titre"]), s(&x["date"]), s(&x["fichier"]), if parts.is_empty() { String::new() } else { format!(", avec {}", parts.join(", ")) }, s(&x["extrait"]))
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        }
        "listes_chercher" => {
            let r = post("/listes", json!({ "op": "search", "query": s(&args["recherche"]), "liste": s(&args["liste"]), "limit": args["limite"] })).await;
            if !r["error"].is_null() {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            text_s(s(&r["text"]))
        }
        "listes_lire" => {
            let r = post("/listes", json!({ "op": "read", "liste": s(&args["liste"]), "element": s(&args["element"]) })).await;
            if !r["error"].is_null() {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            text_s(s(&r["text"]))
        }
        "documents_chercher" => {
            let r = post("/documents", json!({ "query": s(&args["recherche"]), "limit": args["limite"] })).await;
            if !r["error"].is_null() {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            text_s(s(&r["text"]))
        }
        "savoir_chercher" => {
            let r = post("/savoir", json!({ "query": s(&args["recherche"]), "dossier": s(&args["dossier"]), "limit": args["limite"] })).await;
            if !r["error"].is_null() {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            text_s(s(&r["text"]))
        }
        "reunion_lire" => {
            let reference = if args["reference"].as_str().map(|x| !x.is_empty()).unwrap_or(false) { args["reference"].clone() } else { json!("derniere") };
            let r = post("/meetings", json!({ "op": "read", "reference": reference })).await;
            if !r["error"].is_null() {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            text_s(format!("{}{}", s(&r["meeting"]["texte"]), if r["meeting"]["tronque"] == true { "\n(transcription tronquée)" } else { "" }))
        }
        "memoire_effacer" => {
            if args["confirmation"] != true {
                return text_s("Effacement annulé : confirmation requise (confirmation: true).");
            }
            let r = post("/memory", json!({ "op": "clear" })).await;
            if !r["error"].is_null() {
                return text_s(format!("Échec : {}", s(&r["error"])));
            }
            text_s(format!("Mémoire RAG entièrement effacée ({} souvenirs supprimés). Jarvis repart avec une mémoire vierge.", r["deleted"].as_i64().unwrap_or(0)))
        }
        other => text_s(format!("Outil inconnu : {other}")),
    }
}

pub fn run() {
    serve("jarvis", tools(), Rc::new(|name, args| Box::pin(call(name, args))));
}
