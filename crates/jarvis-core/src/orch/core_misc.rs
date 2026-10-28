// Orchestrateur : mémoire RAG, première rencontre (onboarding), journal, vocabulaire, statut et
// empreintes vocales (orchestrator/index.js, lignes 3293-3540).

use std::path::PathBuf;

use serde_json::json;

use super::{extract_birthday, extract_city, extract_name, Orchestrator};
use crate::js::{self, J};

impl Orchestrator {
    pub fn clear_memory(&self) -> J {
        let previous = self.memory.len();
        self.memory.clear();
        // Le profil de dialogue vient des réglages : il survit à la purge du RAG.
        self.sync_profile_memory();
        self.publish_memory();
        self.publish_vocabulary(true);
        self.refresh_daemon();
        self.record("memoire_reinitialisation", json!({ "count": previous }));
        self.log(format!("[mémoire RAG] purge intégrale réussie ({previous} souvenirs supprimés)"));
        json!({ "ok": true, "deleted": previous })
    }

    pub fn start_onboarding(&self) -> String {
        let task_id = self.next_id("onboard");
        *self.onboarding.borrow_mut() = Some(json!({ "taskId": task_id, "step": "name", "data": {} }));
        let a = self.addressing_forms();
        self.emit(json!({
            "type": "create", "id": task_id, "label": "PREMIÈRE RENCONTRE",
            "detail": format!("Faisons connaissance : quel est {} prénom ?", a.ton)
        }));
        let text = format!(
            "Bonjour ! Nous ne nous connaissons pas encore. Pour que je puisse {} au mieux, comment dois-je {} ?",
            a.cod("assister"),
            a.cod("appeler")
        );
        self.emit(json!({ "type": "say", "text": text }));
        task_id
    }

    pub fn handle_onboarding_response(&self, input: &str) -> bool {
        let Some(ob) = self.onboarding.borrow().clone() else { return false };
        let clean = input.trim();
        let step = js::gs(&ob, "step");
        let task_id = js::gs(&ob, "taskId");
        let a = self.addressing_forms();
        let habites = if a.vous { "habitez-vous" } else { "habites-tu" };

        // Annulation ou report du protocole
        if crate::re!(r"(?i)^(?:stop|annule|annuler|plus tard|ignorer|quitter)\b").is_match(clean) {
            self.emit(json!({ "type": "update", "id": task_id, "detail": "Prise de contact reportée" }));
            self.emit(json!({ "type": "finish", "id": task_id, "status": "error" }));
            self.emit(json!({ "type": "say", "text": format!("Très bien, nous ferons connaissance plus tard. Je reste à {} disposition.", a.ton) }));
            *self.onboarding.borrow_mut() = None;
            return true;
        }

        let set = |f: &dyn Fn(&mut J)| {
            if let Some(o) = self.onboarding.borrow_mut().as_mut() {
                f(o);
            }
        };
        let ta = if a.vous { "votre" } else { "ta" };
        match step.as_str() {
            "name" => {
                let name = extract_name(clean);
                set(&|o| {
                    o["data"]["name"] = json!(name);
                    o["step"] = json!("birthday");
                });
                self.memory.add(&format!("Prénom : {name}"), "identité");
                self.publish_memory();
                self.publish_vocabulary(true);
                self.emit(json!({ "type": "update", "id": task_id, "detail": format!("Enchanté {name} ! Quelle est {ta} date de naissance ?") }));
                self.emit(json!({
                    "type": "say",
                    "text": format!("Enchanté {name} ! Quelle est {ta} date de naissance, pour que je puisse penser à {} anniversaire ?", a.ton)
                }));
                true
            }
            "birthday" => {
                let bday = extract_birthday(clean);
                set(&|o| {
                    o["data"]["birthday"] = json!(bday);
                    o["step"] = json!("city");
                });
                self.memory.add(&format!("Date de naissance : {bday}"), "identité");
                self.publish_memory();
                self.emit(json!({ "type": "update", "id": task_id, "detail": format!("C'est noté ! Dans quelle ville {habites} ?") }));
                self.emit(json!({
                    "type": "say",
                    "text": format!("C'est bien noté. Enfin, dans quelle ville {habites}, notamment pour {} donner la météo et l'heure locale ?", a.te)
                }));
                true
            }
            "city" => {
                let city = extract_city(clean);
                set(&|o| o["data"]["city"] = json!(city));
                self.memory.add(&format!("Habite à {city}"), "lieu");
                self.publish_memory();
                self.publish_vocabulary(true);
                let data = self.onboarding.borrow().as_ref().map(|o| o["data"].clone()).unwrap_or_default();
                let name = js::gs(&data, "name");
                self.emit(json!({ "type": "update", "id": task_id, "detail": format!("Configuration terminée : {name} ({city})") }));
                self.emit(json!({ "type": "finish", "id": task_id, "status": "done" }));
                self.emit(json!({
                    "type": "say",
                    "text": format!(
                        "Parfait {name}, j'ai mémorisé toutes {} informations. Tout est prêt, je suis à {} entière disposition !",
                        if a.vous { "vos" } else { "tes" },
                        a.ton
                    )
                }));
                self.record("onboarding_termine", data);
                *self.onboarding.borrow_mut() = None;
                self.refresh_daemon();
                true
            }
            _ => false,
        }
    }

    pub fn publish_memory(&self) {
        let entries = self.memory.list();
        let start = entries.len().saturating_sub(40);
        self.emit(json!({ "type": "memory", "entries": entries[start..] }));
    }

    /// Toute action de Jarvis passe par ici : le journal est le seul point de vérité.
    pub fn record(&self, typ: &str, data: J) -> J {
        let entry = self.history.append(typ, data);
        self.emit(json!({
            "type": "journal",
            "count": entry["seq"],
            "hash": js::slice_chars(&js::gs(&entry, "hash"), 8),
            "ok": true,
            "lastEntry": entry,
            "entries": self.history.search("", 25)
        }));
        entry
    }

    /// Envoie le vocabulaire au service vocal (via l'interface) s'il a changé, ou toujours si force.
    pub fn publish_vocabulary(&self, force: bool) -> J {
        let vocab = self.vocabulary.build();
        let key = vocab.to_string();
        if force || self.last_vocabulary.borrow().as_deref() != Some(key.as_str()) {
            *self.last_vocabulary.borrow_mut() = Some(key);
            let mut ev = json!({ "type": "vocabulary" });
            js::assign(&mut ev, vocab.clone());
            self.emit(ev);
        }
        vocab
    }

    pub fn voiceprint_path(&self) -> PathBuf {
        let exe_data = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("data").join("voiceprint.json")));
        let candidates: Vec<PathBuf> = [
            Some(self.data_dir.join("voiceprint.json")),
            exe_data,
            Some(self.workspace.join("data").join("voiceprint.json")),
        ]
        .into_iter()
        .flatten()
        .collect();
        candidates.iter().find(|p| p.exists()).cloned().unwrap_or_else(|| candidates[0].clone())
    }

    pub fn get_speaker_status(&self) -> J {
        let p = self.voiceprint_path();
        if let Some(d) = js::read_json(&p) {
            let mut speakers: Vec<J> = js::arr(&d["speakers"])
                .iter()
                .map(|s| {
                    let primary = js::truthy(&s["isPrimary"]);
                    let learned = js::gn(s, "learnedCount").filter(|n| *n != 0.0).map(|n| n as i64).unwrap_or_else(|| s["learned"].as_array().map(|l| l.len() as i64).unwrap_or(0));
                    let configs: Vec<String> = s["variants"].as_object().map(|m| m.keys().filter(|k| *k != "*").cloned().collect()).unwrap_or_default();
                    let typ = js::gs(s, "type");
                    json!({
                        "id": Some(js::gs(s, "id")).filter(|x| !x.is_empty()).unwrap_or_else(|| "johann".into()),
                        "name": Some(js::gs(s, "name")).filter(|x| !x.is_empty()).unwrap_or_else(|| "Johann".into()),
                        "samplesCount": js::gn(s, "samplesCount").filter(|n| *n != 0.0).unwrap_or(3.0),
                        "enrolledAt": if js::truthy(&s["enrolledAt"]) { s["enrolledAt"].clone() } else { J::Null },
                        "isPrimary": primary,
                        // Lien avec l'utilisateur principal (fille, femme, ami…) et phrases apprises depuis le HUD.
                        "type": if !typ.is_empty() { typ } else if primary { "moi".into() } else { "autre".into() },
                        "learned": learned,
                        // v6 : configurations audio enregistrées (casque, haut-parleurs).
                        "configs": configs
                    })
                })
                .collect();
            // Rétrocompatibilité : speakers vide mais empreinte racine présente.
            if speakers.is_empty() && js::truthy(&d["enrolled"]) {
                speakers.push(json!({
                    "id": "johann", "name": "Johann",
                    "samplesCount": js::gn(&d, "samplesCount").filter(|n| *n != 0.0).unwrap_or(3.0),
                    "enrolledAt": if js::truthy(&d["enrolledAt"]) { d["enrolledAt"].clone() } else { J::Null },
                    "isPrimary": true
                }));
            }
            let primary = speakers.iter().find(|s| js::gb(s, "isPrimary")).or_else(|| speakers.first()).cloned();
            let enrolled_at = primary.as_ref().map(|p| p["enrolledAt"].clone()).filter(js::truthy).or_else(|| Some(d["enrolledAt"].clone()).filter(js::truthy)).unwrap_or(J::Null);
            let samples = primary.as_ref().and_then(|p| js::gn(p, "samplesCount")).filter(|n| *n != 0.0).or_else(|| js::gn(&d, "samplesCount")).unwrap_or(0.0);
            let doubt = js::gn(&d, "doubtMargin").filter(|n| n.is_finite()).unwrap_or(0.15);
            return json!({
                "available": true,
                "enrolled": js::truthy(&d["enrolled"]) || !speakers.is_empty(),
                "enabled": if d.get("enabled").is_some() { js::truthy(&d["enabled"]) } else { true },
                "threshold": js::gn(&d, "threshold").filter(|n| *n != 0.0).unwrap_or(0.65),
                "enrolledAt": enrolled_at,
                "samplesCount": samples,
                "speakersCount": speakers.len(),
                "primarySpeaker": primary.as_ref().map(|p| p["name"].clone()).unwrap_or(J::Null),
                "speakers": speakers,
                "thresholds": if js::truthy(&d["thresholds"]) { d["thresholds"].clone() } else { json!({}) },
                "mode": if js::gs(&d, "mode") == "signal" { "signal" } else { "reject" },
                "doubtMargin": doubt,
                "nearMiss": js::gn(&d, "nearMiss").filter(|n| n.is_finite()).map(|n| n.clamp(0.0, 0.2)).unwrap_or(0.12),
                "autoLearn": d["autoLearn"] != json!(false),
                "learning": d["learning"] != json!(false)
            });
        }
        json!({
            "available": true, "enrolled": false, "enabled": true, "threshold": 0.65, "enrolledAt": null,
            "samplesCount": 0, "speakersCount": 0, "speakers": [], "primarySpeaker": null
        })
    }

    pub fn publish_status(&self) {
        let mut ev = self.status.snapshot();
        ev["contextMute"] = self.context_mute.get_state();
        ev["speaker"] = self.get_speaker_status();
        self.emit(ev);
    }

    pub fn publish_journal(&self) {
        let integrity = self.history.verify();
        self.emit(json!({
            "type": "journal",
            "count": integrity["count"],
            "hash": js::slice_chars(&self.history.last.borrow(), 8),
            "ok": integrity["ok"],
            "reason": integrity["reason"],
            "entries": self.history.search("", 25)
        }));
    }
}

