// Orchestrateur : traitement d'une demande (submit), intentions locales, corrections apprises, dictée,
// macros, file des demandes pour l'IA et accusés de réception (orchestrator/index.js, lignes 1249 à 1988).

use std::cell::Cell;
use std::rc::Rc;

use serde_json::{json, Value};

use super::intent::Intent;
use super::policy::shorten;
use super::{phrases, speakable, Fut, Orchestrator, QueuedTask};
use crate::js::{self, J};

fn fold_cancel(s: &str) -> String {
    let t = js::fold_lower(s);
    let t = crate::re!(r"['’\-?!,;:.…]").replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

fn cancel_countdown_re() -> &'static regex::Regex {
    crate::re!(r"^(?:jarvis )?(?:annule|annuler|annulez|stop|stoppe|arrete|arreter|arretez|non|laisse tomber|ne fais pas ca|surtout pas|ca suffit|halte)(?: (?:ca|tout|le compte a rebours|le decompte|l extinction|le redemarrage|la mise en veille|l hibernation|le verrouillage|l action|la programmation|le chrono|jarvis|s il te plait|stp))*$")
}

impl Orchestrator {
    /// meta.source : « typed » (clavier) ou « voice ». Sans origine déclarée, la demande est traitée
    /// comme vocale : elle ne peut jamais valider un code tapé.
    pub fn submit(self: &Rc<Self>, text: String, meta: J) -> Fut<()> {
        let me = self.clone();
        Box::pin(async move { me.submit_inner(text, meta).await })
    }

    async fn submit_inner(self: Rc<Self>, text: String, meta: J) {
        let clean = text.trim().to_string();
        if clean.is_empty() {
            return;
        }
        let from_voice = js::gs(&meta, "source") != "typed";
        // Une programmation qui lance une commande n'est pas l'utilisateur : elle ne réveille pas les annonces.
        if js::gs(&meta, "source") != "programmation" {
            self.announcer.user_spoke();
        }
        // Même ligne de card cliquée deux fois de suite : une seule demande.
        if js::gs(&meta, "from") == "widget" {
            let dup = self.last_widget_request.borrow().as_ref().map(|(t, at)| *t == clean && js::now_ms() - at < 2000).unwrap_or(false);
            if dup {
                return;
            }
            *self.last_widget_request.borrow_mut() = Some((clean.clone(), js::now_ms()));
        }
        // Une card ouverte ou fermée plusieurs fois par ses boutons dans les 20 secondes : la réponse n'est dite que la
        // première fois (la card s'ouvre quand même).
        struct QuietGuard<'a>(&'a Cell<bool>);
        impl Drop for QuietGuard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _quiet = QuietGuard(&self.quiet_say);
        if js::gs(&meta, "from") == "widget" {
            let now = js::now_ms();
            let mut seen = self.widget_announced.borrow_mut();
            match seen.get(&clean) {
                Some(t) if now - *t < 20_000 => self.quiet_say.set(true),
                _ => {
                    seen.insert(clean.clone(), now);
                }
            }
        }
        let speaker = Some(js::gs(&meta["speaker"], "name")).filter(|s| !s.is_empty());
        *self.current_speaker.borrow_mut() = speaker.clone();
        *self.last_request_source.borrow_mut() = if from_voice { "voice".into() } else { "typed".into() };

        // Un code de validation tapé n'est jamais journalisé en clair.
        let is_code = crate::re!(r"^\d{4}$").is_match(&clean) && self.pending.borrow().iter().any(|(_, e)| js::truthy(&e["code"]));
        let source = if js::gs(&meta, "source") == "telegram" { "telegram" } else if from_voice { "voix" } else { "clavier" };
        self.record("demande", json!({ "texte": if is_code { "[code de validation]".to_string() } else { clean.clone() }, "source": source, "locuteur": speaker }));

        // Émotion de la phrase (score 1..10), transmise au cœur et au personnage.
        if !is_code {
            if let Some(emo) = self.emotions.evaluate(&clean) {
                self.emit(json!({ "type": "emotion", "score": emo["score"], "action": emo["action"], "name": emo["name"], "text": clean }));
            }
        }

        // 00. Compte à rebours d'une programmation affiché : « annule », « stop », « non » l'arrêtent.
        if self.schedules.active_countdown().is_some() && cancel_countdown_re().is_match(&fold_cancel(&clean)) {
            let item = self.schedules.cancel_countdown(if from_voice { "voix" } else { "clavier" });
            self.emit(json!({ "type": "say", "text": if item.is_some() { "Annulé. Rien ne sera fait." } else { "Il n’y a plus rien à annuler." } }));
            return;
        }
        // 0. Dictée : chaque phrase dite est écrite dans le document, sans IA.
        if self.dictation.borrow().is_some() && from_voice && self.handle_dictation(&clean).await {
            return;
        }
        // 0 a. Prise de notes : la phrase est écrite dans la card (« Jarvis, … » reste une commande).
        if self.notes.listening.get() && from_voice && self.notes.handle_voice(&clean, js::gb(&meta, "wake")).await {
            return;
        }
        // 0 a bis. Terminal au premier plan : la phrase y est écrite.
        let doubt = js::gb(&meta, "speakerDoubt");
        let has_terminal = self.terminal_voice.target.borrow().is_some();
        let rejected = js::gb(&meta, "speakerRejected");
        if from_voice && has_terminal && self.terminal_voice.handle(&clean, doubt, rejected).await {
            return;
        }
        if from_voice && !has_terminal && !rejected && self.terminal_voice.handle_recent(&clean, doubt).await {
            return;
        }
        // Voix douteuse acceptée seulement pour un terminal (au premier plan, ou relais vers Claude).
        if doubt {
            if from_voice && self.tmux_voice.relay.borrow().is_some() && self.tmux_voice.handle_relay(&clean, true).await {
                return;
            }
            self.record("demande_ignoree", json!({ "texte": js::slice_chars(&clean, 300), "motif": "voix douteuse hors terminal" }));
            return;
        }

        // 0 bis. Correction : « non, j'ai dit mets de la musique », « /corrige … ».
        let said = crate::re!(r"(?i)^(?:non[,.!]?\s+)?(?:j['’]ai dit|je disais|j['’]ai demandé|je voulais dire|c['’]est|c['’]etait|c['’]était)\s*[:,]?\s+(.+)$").captures(&clean).or_else(|| crate::re!(r"(?i)^/corrige\s+(.+)$").captures(&clean)).map(|m| m[1].trim().to_string());
        let previous = self.last_voice_utterance.borrow().clone();
        if let (Some(right), Some((prev, at))) = (&said, &previous) {
            if js::now_ms() - at < 120_000 {
                let learned = self.learn_from_correction(prev, right);
                *self.last_voice_utterance.borrow_mut() = None;
                if js::gb(&learned, "retenu") {
                    self.emit(json!({ "type": "say", "text": format!("Correction enregistrée : « {} » devient « {} ».", js::gs(&learned, "entendu"), js::gs(&learned, "correct")) }));
                }
                self.submit(right.clone(), meta).await;
                return;
            }
        }
        if from_voice {
            *self.last_voice_utterance.borrow_mut() = Some((clean.clone(), js::now_ms()));
        }
        // Mémoire automatique : seules les phrases de l'utilisateur (pas un code, pas une autre voix) qui parlent de lui.
        if !is_code && !rejected {
            self.memory_extract.note(&clean);
        }

        // 0 ter. tmux : relais actif (la phrase est tapée dans le terminal).
        if from_voice && self.tmux_voice.relay.borrow().is_some() && self.tmux_voice.handle_relay(&clean, false).await {
            return;
        }
        // 0 quater. Aide aux commandes ; « oui » juste après exécute la commande proposée.
        let idle = self.pending.borrow().is_empty() && self.waiting_reply.borrow().is_none();
        if idle {
            if let Some(s) = self.command_help.take_suggestion(&clean) {
                self.submit(s, meta).await;
                return;
            }
        }
        if let Some(i) = self.command_help.match_(&clean) {
            self.run_intent(i, &clean).await;
            return;
        }
        if let Some(i) = self.tmux_voice.match_(&clean, false) {
            self.run_intent(i, &clean).await;
            return;
        }
        // Changement de verbosité du vibe coding (« mode ultra court », « sois plus explicatif »).
        if self.vibe_coding.handle_verbosity_switch(&clean) {
            return;
        }
        // 1. Commandes directes de configuration (/moteur, /voix, /zones…).
        if let Some(command) = self.status.parse(&clean) {
            self.usage_daily.add_local("local");
            self.apply_command(command, false).await;
            self.refresh_daemon();
            self.publish_vocabulary(false);
            return;
        }

        // 2. Interruption ou annulation explicite.
        if let Some(m) = crate::re!(r"(?i)^(?:/stop|/cancel|/annule|/interromps|stop|arrete|arrête|annule|interromps|annuler|quitte)(?:\s+(?:tout|tous|la\s+tache|la\s+tâche|le\s+fond|en\s+fond|([a-z0-9_-]+)))?$").captures(&clean) {
            let target = m.get(1).map(|x| x.as_str().trim().to_lowercase()).unwrap_or_default();
            if target == "tout" || target == "tous" || crate::re!(r"(?i)^(?:stop\s+tout|annule\s+tout)$").is_match(&clean) {
                self.stop_all("commande");
                self.emit(json!({ "type": "say", "text": "Toutes les tâches ont été arrêtées." }));
                return;
            }
            let mut stopped = false;
            if target == "fond" || crate::re!(r"(?i)fond").is_match(&clean) {
                let bg: Vec<String> = self.running.borrow().iter().filter(|(_, e)| e.background).map(|(k, _)| k.clone()).collect();
                for id in bg {
                    self.cancel(&id);
                    stopped = true;
                }
                if stopped {
                    self.emit(json!({ "type": "say", "text": "Tâche de fond interrompue." }));
                    return;
                }
            }
            if !target.is_empty() && self.is_running(&target) {
                self.cancel(&target);
                self.emit(json!({ "type": "say", "text": format!("Tâche {target} interrompue.") }));
                return;
            }
            if self.vibe_coding.has_pending_confirmation() {
                self.vibe_coding.handle_confirmation("annule").await;
                stopped = true;
            }
            let pending: Vec<String> = self.pending.borrow().iter().map(|(k, _)| k.clone()).collect();
            if !pending.is_empty() {
                for id in pending {
                    self.deny(&id);
                }
                stopped = true;
            }
            if self.close_waiting_reply(json!({ "error": "tâche interrompue par l’utilisateur" })) {
                stopped = true;
            }
            if self.waiting_vocab_clarification.borrow_mut().take().is_some() {
                self.emit(json!({ "type": "vocab_learned", "entendu": null, "correct": null }));
                stopped = true;
            }
            if self.smart_lists.cancel_creation() {
                stopped = true;
            }
            if self.collections.cancel_conversation() {
                stopped = true;
            }
            if let Some(ob) = self.onboarding.borrow_mut().take() {
                self.emit(json!({ "type": "update", "id": ob["taskId"], "detail": "Protocole interrompu" }));
                self.emit(json!({ "type": "finish", "id": ob["taskId"], "status": "error" }));
                stopped = true;
            }
            let to_cancel = {
                let r = self.running.borrow();
                r.iter().rev().find(|(_, e)| !e.background).map(|(k, _)| k.clone()).or_else(|| r.last().map(|(k, _)| k.clone()))
            };
            if let Some(id) = to_cancel {
                if self.cancel(&id) {
                    stopped = true;
                }
            }
            if stopped {
                self.emit(json!({ "type": "say", "text": "Tâche interrompue." }));
                return;
            }
        }

        // 2bis 0. Réponse à une demande de clarification de mot (« je n'ai pas compris X »).
        let vocab = self.waiting_vocab_clarification.borrow().clone().filter(|w| js::now_ms() - w["at"].as_i64().unwrap_or(0) < 60000);
        if let Some(w) = vocab {
            *self.waiting_vocab_clarification.borrow_mut() = None;
            let heard = js::gs(&w, "heard");
            let full = js::gs(&w, "fullPhrase");
            self.emit(json!({ "type": "vocab_learned", "entendu": heard, "correct": clean }));
            if crate::re!(r"(?i)^(?:annule|annuler|laisse|laisse tomber|non|echap|stop|passe)\b").is_match(&clean) {
                self.emit(json!({ "type": "say", "text": "D’accord, j’annule." }));
                return;
            }
            if crate::re!(r"(?i)^(?:transmets(?: a l'ia)?|ia|c'est pour l'ia|demande a l'ia)\b").is_match(&clean) {
                let mut m = meta.clone();
                m["currentSpeaker"] = json!(speaker);
                self.enqueue_ai(full, m);
                return;
            }
            let right_input = clean.trim().to_string();
            let mut right_word = right_input.clone();
            let mut corrected = right_input.clone();
            if let Some(p) = crate::re!(r"(?i)^(?:liste|affiche|montre|ouvre|ferme|lance|mets|cherche)\s+").find(&full) {
                let prefix = p.as_str();
                if !right_input.to_lowercase().starts_with(&prefix.to_lowercase()) {
                    corrected = format!("{prefix}{right_input}");
                } else {
                    right_word = right_input[prefix.len()..].trim().to_string();
                }
            }
            self.vocabulary.correct(&heard, &right_word);
            self.publish_vocabulary(true);
            self.record("vocabulaire_correction_interactive", json!({ "entendu": heard, "correct": right_word, "phraseOriginale": full, "phraseCorrigee": corrected }));
            self.emit(json!({ "type": "say", "text": format!("D'accord ! J'ai associé « {heard} » à « {right_word} » dans mon vocabulaire.") }));
            self.submit(corrected, meta).await;
            return;
        }

        // 2bis - 1. Réponse au raffinement interactif d'une liste intelligente.
        if self.smart_lists.creation_fresh(90000) && self.router.route(&clean, false).is_none() {
            if let Some(res) = self.smart_lists.handle_answer(&clean) {
                if js::truthy(&res["say"]) {
                    self.emit(json!({ "type": "say", "text": res["say"] }));
                }
                return;
            }
        }

        // 2bis - 1 ter. Présentation animée en cours : pilotage (« suivant », « stop »…) ou question sur le sujet ;
        // une commande locale reconnue (musique, volume…) garde son chemin.
        if self.savoir.presenting() {
            let routed = self.router.route(&clean, false).filter(|i| !i.ignored).is_some();
            if self.savoir.on_utterance(&super::router::fold(&clean), &clean, routed) {
                return;
            }
        } else if self.savoir.exploring().is_some() {
            // Explorateur 3D ouvert (hors présentation) : commandes 3D, pièce à montrer, question sur le produit.
            let routed = self.router.route(&clean, false).filter(|i| !i.ignored).is_some();
            if self.savoir.on_explore_utterance(&super::router::fold(&clean), &clean, routed) {
                return;
            }
        }

        // 2bis - 1 bis. Création, modification ou suppression d'une liste à fiches en cours de conversation.
        if self.collections.conversation_fresh(90000) && self.router.route(&clean, false).filter(|i| !i.ignored && !i.quiet).is_none() {
            if let Some(res) = self.collections.handle_answer(&clean) {
                if js::truthy(&res["say"]) {
                    self.emit(json!({ "type": "say", "text": res["say"] }));
                }
                return;
            }
        }

        // 2bis. Réponse à une question de précision (une commande reconnue reste une demande nouvelle).
        if self.waiting_reply.borrow().is_some() && self.router.route(&clean, false).is_none() {
            let w = self.waiting_reply.borrow_mut().take().unwrap();
            self.record("clarification_repondue", json!({ "tache": w.task_id, "question": w.question, "reponse": clean }));
            self.emit(json!({ "type": "update", "id": w.task_id, "status": "running", "detail": format!("Réponse : {}", shorten(&clean, 40)) }));
            self.log(format!("réponse utilisateur reçue ({}) : \"{clean}\"", w.task_id));
            w.timer.clear();
            if let Some(tx) = w.tx {
                let _ = tx.send(json!({ "answer": clean }));
            }
            return;
        }

        // 2ter. Prise de contact (premier lancement ou mémoire purgée).
        let onboarding = self.onboarding.borrow().as_ref().map(|o| js::truthy(&o["step"])).unwrap_or(false);
        if onboarding && self.router.route(&clean, false).is_none() && self.handle_onboarding_response(&clean) {
            return;
        }

        // 2quater. Confirmation du vibe coding.
        if self.vibe_coding.has_pending_confirmation() && self.vibe_coding.handle_confirmation(&clean).await {
            return;
        }

        // 3. Validation d'actions en attente.
        if !self.pending.borrow().is_empty() {
            if crate::re!(r"(?i)^(?:non|refuse|pas question)\b").is_match(&clean) {
                let ids: Vec<String> = self.pending.borrow().iter().map(|(k, _)| k.clone()).collect();
                for id in ids {
                    self.deny(&id);
                }
                self.emit(json!({ "type": "say", "text": "Action annulée." }));
                return;
            }
            if crate::re!(r"(?i)^(?:oui|vas[\s-]y|d['’]accord|valide|je valide|confirme|accepte|c['’]est bon|ok)\b").is_match(&clean) {
                let voice = self.pending.borrow().iter().find(|(_, e)| js::gs(e, "validationType") == "voice").map(|(k, _)| k.clone());
                match voice {
                    Some(id) => {
                        self.approve(&id);
                        self.emit(json!({ "type": "say", "text": "Action validée." }));
                    }
                    None => self.emit(json!({ "type": "say", "text": "Cette action est critique, elle exige la saisie du code affiché à l'écran." })),
                }
                return;
            }
            // Code à 4 chiffres : dicté (empreinte vocale exigée) ou tapé.
            if from_voice {
                if let Some(r) = self.code_entry.voice(&clean, &meta) {
                    if js::gb(&r, "refused") {
                        self.emit(json!({ "type": "say", "text": "Je ne reconnais pas ta voix : clique ou tape le code affiché à l'écran." }));
                    }
                    return;
                }
            } else if let Some(m) = crate::re!(r"^\s*(\d{1,4})\s*$").captures(&clean) {
                if self.code_entry.has_current() {
                    self.code_entry.digits(&m[1], None, "clavier");
                    return;
                }
            }
        }

        if crate::re!(r"(?i)^(?:nouvelle\s+session|nouvelle\s+conversation)$").is_match(&clean) || (crate::re!(r"(?i)^oublie\s+tout$").is_match(&clean)) {
            self.reset_conversation();
            self.refresh_daemon();
            self.emit(json!({ "type": "say", "text": "Nouvelle session ouverte." }));
            return;
        }

        // 3 bis. Mode économie strict : réponse à « Je transmets à l'IA ? ».
        let waiting_ai = self.pending_ai_confirm.borrow_mut().take().filter(|(_, _, at)| js::now_ms() - at < 60000);
        if let Some((text, m, at)) = waiting_ai {
            if crate::re!(r"(?i)^(?:oui|vas[\s-]y|d['’]accord|ok|transmets|envoie)\b").is_match(&clean) {
                self.enqueue_ai(text, m);
                return;
            }
            if crate::re!(r"(?i)^(?:non|laisse|annule|pas la peine)\b").is_match(&clean) {
                self.emit(json!({ "type": "say", "text": "Très bien, je ne transmets pas." }));
                return;
            }
            let _ = at;
        }

        // 3 ter. Mission en plusieurs étapes : « reprends / abandonne la mission ».
        if let Some(cmd) = super::missions::Missions::command(&super::router::fold(&clean)) {
            self.missions.handle_command(cmd).await;
            return;
        }
        // 4. Macros personnelles : phrase exacte choisie par l'utilisateur.
        if let Some(macro_) = self.macros.match_(&clean) {
            self.run_macro(&macro_, from_voice).await;
            return;
        }
        // 4 bis. Routeur d'intentions local (avec reprise du routage spéculatif).
        let mut intent: Option<Intent> = None;
        if let Some((spec_text, spec_intent, at)) = self.speculative.borrow_mut().take() {
            if js::now_ms() - at < 4000 {
                let (st, ct) = (spec_text.to_lowercase(), clean.to_lowercase());
                let prefix_ok = !spec_intent.free_text && (ct.starts_with(&st) || st.starts_with(&ct));
                if ct == st || prefix_ok {
                    self.log(format!("[routage spéculatif] intention réutilisée ({}) pour \"{}\"", spec_intent.kind, shorten(&clean, 60)));
                    intent = Some(spec_intent);
                }
            }
        }
        if intent.is_none() {
            intent = self.router.route(&clean, from_voice);
        }
        if let Some(i) = intent {
            self.run_intent(i, &clean).await;
            return;
        }
        // 4 ter. Routeur sémantique : reformulation libre d'une commande locale connue.
        if let Some((canonical, score)) = self.semantic_router.match_(&clean).await {
            if let Some(via) = self.router.route(&canonical, false).filter(|i| !i.ignored) {
                self.log(format!("routeur sémantique : \"{}\" → {canonical} ({score:.2})", shorten(&clean, 60)));
                self.run_intent_with(via, &clean, Some((canonical, score))).await;
                return;
            }
        }
        // 4 quater. Réponse déjà donnée récemment à la même question factuelle.
        if let Some(cached) = self.answer_cache.get(&clean) {
            let answer = cached.answer.clone();
            self.record("reponse_cache", json!({ "texte": clean, "reponse": answer, "age_s": ((js::now_ms() - cached.at) as f64 / 1000.0).round() as i64 }));
            self.usage_daily.add_local("local");
            self.publish_status();
            self.emit(json!({ "type": "say", "text": speakable(&answer), "answer": true }));
            return;
        }
        // 4 quater bis. Mission : étapes locales et étapes pour l'IA mêlées, faites dans l'ordre.
        if let Some(steps) = self.missions.plan(&clean) {
            let me = self.clone();
            let c2 = clean.clone();
            js::spawn(async move { me.missions.start(&c2, steps).await });
            return;
        }
        let mut ai_meta = meta.clone();
        ai_meta["currentSpeaker"] = json!(speaker);
        // 4 quinquies. Mode strict : confirmation avant tout appel à une IA externe.
        if self.status.d.borrow().economy_level == "strict" {
            *self.pending_ai_confirm.borrow_mut() = Some((clean.clone(), ai_meta, js::now_ms()));
            self.emit(json!({ "type": "say", "text": "Je ne sais pas le faire seul. Je transmets à l'IA ?" }));
            return;
        }
        // 4 sexies. Vibe coding : modification d'un élément pointé à l'écran.
        if self.vibe_coding.is_vibe_utterance(&clean) {
            self.vibe_coding.start_session(&clean, ai_meta).await;
            return;
        }
        // 4 septies. Mot ou commande orale non compris : on demande de l'écrire pour l'associer.
        let action = if from_voice {
            crate::re!(r"(?i)^(?:liste|affiche|montre|donne|ouvre|ferme|lance|demarre|démarre|mets|cherche|active|coupe|card|historique)\s+(?:(?:moi|nous)\s+)?(?:(?:la|le|les|des?|un|une)\s+)?([a-z0-9_-]+(?:\s+[a-z0-9_-]+)?)$").captures(clean.trim()).map(|m| m[1].to_string())
        } else {
            None
        };
        let isolated = from_voice && clean.split_whitespace().count() <= 2 && !crate::re!(r"(?i)^(?:pourquoi|comment|qu['’]est|qui|combien|peux|explique|aide|bonjour|merci|salut)\b").is_match(&clean);
        if (action.is_some() || isolated) && self.waiting_reply.borrow().is_none() {
            let word = action.unwrap_or_else(|| clean.clone()).trim().to_string();
            *self.waiting_vocab_clarification.borrow_mut() = Some(json!({ "heard": word, "fullPhrase": clean, "at": js::now_ms() }));
            self.record("vocabulaire_incompris", json!({ "entendu": word, "phrase": clean }));
            self.emit(json!({ "type": "vocab_ask", "heard": word, "fullPhrase": clean }));
            self.emit(json!({ "type": "say", "text": format!("Je n'ai pas compris « {word} ». Tu peux me l'écrire pour que je l'associe.") }));
            return;
        }
        self.enqueue_ai(clean, ai_meta);
    }

    /// Exécute une intention locale (sans IA) : carte d'action, journal, compteur, réponse parlée.
    pub async fn run_intent(self: &Rc<Self>, intent: Intent, clean: &str) -> Option<J> {
        self.run_intent_with(intent, clean, None).await
    }

    pub async fn run_intent_with(self: &Rc<Self>, intent: Intent, clean: &str, semantic: Option<(String, f32)>) -> Option<J> {
        let id = self.next_id("local");
        if !intent.quiet {
            self.emit(json!({ "type": "create", "id": id, "label": intent.label, "detail": intent.detail }));
        }
        let result = intent.run().await;
        if intent.ignored {
            self.record("demande_ignoree", json!({ "texte": clean, "motif": intent.reason }));
            self.log(format!("demande ignorée ({}) : \"{}\"", intent.reason, shorten(clean, 80)));
        } else {
            let mut data = json!({ "texte": clean, "type": intent.kind, "reponse": result["say"] });
            if let Some((c, s)) = &semantic {
                data["sens"] = json!(c);
                data["score"] = json!(((*s as f64) * 100.0).round() / 100.0);
            }
            self.record("intention_locale", data);
        }
        self.usage_daily.add_local(if intent.ignored { "ignored" } else { "local" });
        self.publish_status();
        let say = js::gs(&result, "say");
        let display = js::gs(&result, "display");
        let negative = regex::Regex::new(super::failures::NEGATIVE_REPLY).map(|r| r.is_match(&say)).unwrap_or(false);
        if !intent.ignored && (negative || crate::re!(r"(?i)^échec$").is_match(&display)) {
            let erreur = if say.is_empty() { display.clone() } else { say.clone() };
            self.failures.add(json!({ "kind": "local", "demande": clean, "intention": intent.kind, "erreur": erreur, "source": *self.last_request_source.borrow() }));
        }
        if !intent.quiet {
            self.emit(json!({ "type": "finish", "id": id, "status": "done" }));
        }
        if !say.is_empty() {
            self.emit(json!({ "type": "say", "text": say, "answer": true, "force": intent.kind == "repeat" }));
            if intent.kind != "repeat" {
                *self.last_spoken.borrow_mut() = say;
            }
        }
        Some(result)
    }

    // --- Corrections apprises -------------------------------------------------------------------------

    /// Compare la phrase mal comprise et la phrase corrigée ; seules les différences sûres sont apprises.
    pub fn learn_from_correction(&self, heard: &str, right: &str) -> J {
        let words = |t: &str| -> Vec<String> {
            let t = crate::re!(r"(?i)^jarvis[,\s]+").replace(t, "");
            let t = crate::re!(r"[.,!?;:]+").replace_all(&t, " ");
            t.split_whitespace().map(String::from).collect()
        };
        let (a, b) = (words(heard), words(right));
        let mut start = 0;
        while start < a.len() && start < b.len() && a[start].to_lowercase() == b[start].to_lowercase() {
            start += 1;
        }
        let (mut end_a, mut end_b) = (a.len() as isize - 1, b.len() as isize - 1);
        while end_a >= start as isize && end_b >= start as isize && a[end_a as usize].to_lowercase() == b[end_b as usize].to_lowercase() {
            end_a -= 1;
            end_b -= 1;
        }
        let slice = |v: &[String], e: isize| if e >= start as isize { v[start..=(e as usize)].join(" ") } else { String::new() };
        let (wrong, good) = (slice(&a, end_a), slice(&b, end_b));
        const STOP: &[&str] = &[
            "le", "la", "les", "un", "une", "des", "du", "de", "d", "l", "et", "ou", "a", "à", "en", "au", "aux", "ce", "cet", "cette", "ces", "mon", "ton", "son", "ma", "ta", "sa", "mes", "tes", "ses", "notre", "votre", "leur", "je", "tu", "il", "elle", "on", "nous", "vous", "ils", "elles", "est", "es", "suis", "sommes", "sont", "ai", "as", "avons", "avez", "ont", "met", "mets", "fais", "fait", "dis", "dit", "par", "sur", "dans", "pour", "avec", "sans", "sous",
        ];
        let proper = good.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        let stop = STOP.contains(&good.to_lowercase().as_str()) || STOP.contains(&wrong.to_lowercase().as_str());
        let (wl, gl) = (wrong.encode_utf16().count(), good.encode_utf16().count());
        let safe_word = gl >= 3 && wl >= 2 && !stop;
        let safe = !wrong.is_empty() && !good.is_empty() && wl <= 40 && gl <= 40 && end_a - start as isize + 1 <= 4 && end_b - start as isize + 1 <= 4 && (wrong.contains(' ') || good.contains(' ') || proper || safe_word);
        let mut retenu = false;
        if safe {
            retenu = self.vocabulary.correct(&wrong, &good).is_some();
            if retenu {
                self.publish_vocabulary(true);
            }
        }
        let learned = json!({ "entendu": wrong, "correct": good, "retenu": retenu });
        let mut data = json!({ "avant": heard, "apres": right });
        js::assign(&mut data, learned.clone());
        self.record("correction_orale", data);
        learned
    }

    // --- Dictée --------------------------------------------------------------------------------------

    pub fn start_dictation(&self, target: &str) {
        *self.dictation.borrow_mut() = Some(json!({ "target": target, "since": js::now_ms(), "lastAt": js::now_ms(), "typed": 0 }));
        self.record("dictee_debut", json!({ "cible": target }));
    }

    pub fn stop_dictation(&self, reason: &str) {
        let Some(d) = self.dictation.borrow_mut().take() else { return };
        self.record("dictee_fin", json!({ "cible": d["target"], "phrases": d["typed"], "motif": reason }));
    }

    /// Tape la phrase (même politique que l'outil saisir_texte). Retourne le message à dire.
    pub async fn dictate(self: &Rc<Self>, text: &str) -> Option<String> {
        let typed = super::desktop::spoken_punctuation(text);
        let me = self.clone();
        let r = self
            .local_tool("mcp__system__saisir_texte", json!({ "texte": typed }), Box::new(move |p: J| {
                Box::pin(async move {
                    let t = js::gs(&p, "texte");
                    let needs_space = me.dictation.borrow().as_ref().map(|d| js::gn_or(d, "typed", 0.0) > 0.0).unwrap_or(false) && !crate::re!(r"^[\n.,;:!?…»]").is_match(&t);
                    me.desktop.type_text(&if needs_space { format!(" {t}") } else { t }).await
                })
            }))
            .await;
        if js::gb(&r, "ok") {
            if let Some(d) = self.dictation.borrow_mut().as_mut() {
                d["typed"] = json!(js::gn_or(d, "typed", 0.0) as i64 + 1);
                d["lastAt"] = json!(js::now_ms());
            }
        }
        if js::gb(&r, "refus") {
            return Some("Clique dans ton document, puis redis ta phrase.".into());
        }
        if !js::truthy(&r) || r["ok"] == json!(false) {
            return Some(if js::gb(&r, "denied") { "Saisie refusée par les réglages de sécurité.".into() } else { "Je n'ai pas pu écrire dans le document.".into() });
        }
        None
    }

    pub async fn correct_document(self: &Rc<Self>, whole: bool) -> String {
        let me = self.clone();
        let r = self
            .local_tool("mcp__system__saisir_texte", json!({ "texte": if whole { "[correction du document]" } else { "[correction de la sélection]" } }), Box::new(move |_| Box::pin(async move { me.desktop.correct(whole).await })))
            .await;
        if js::gb(&r, "refus") {
            return "Clique dans ton document, puis redemande la correction.".into();
        }
        if !js::truthy(&r) || r["ok"] == json!(false) {
            return if js::truthy(&r["error"]) { format!("Correction impossible : {}", js::gs(&r, "error")) } else { "Correction impossible.".into() };
        }
        if js::gb(&r, "vide") {
            return "Le document est vide.".into();
        }
        let n = js::gn_or(&r, "corrections", 0.0) as i64;
        if n == 0 {
            return "Je n'ai trouvé aucune faute.".into();
        }
        let s = if n > 1 { "s" } else { "" };
        format!("{n} correction{s} appliquée{s}. Ctrl+Z pour annuler.")
    }

    /// true si la phrase a été traitée par la dictée.
    async fn handle_dictation(self: &Rc<Self>, clean: &str) -> bool {
        let last = self.dictation.borrow().as_ref().map(|d| d["lastAt"].as_i64().unwrap_or(0)).unwrap_or(0);
        if js::now_ms() - last > 5 * 60 * 1000 {
            self.stop_dictation("inactivite");
            return false;
        }
        let t = crate::re!(r"^jarvis ").replace(&fold_cancel(clean), "").into_owned();
        self.usage_daily.add_local("local");
        let touch = || {
            if let Some(d) = self.dictation.borrow_mut().as_mut() {
                d["lastAt"] = json!(js::now_ms());
            }
        };
        if crate::re!(r"^(?:fin de (?:la )?dictee|(?:arrete|stoppe|stop|termine|quitte) (?:la )?dictee|j ai fini|c est fini|stop|fin)$").is_match(&t) {
            let n = self.dictation.borrow().as_ref().map(|d| js::gn_or(d, "typed", 0.0) as i64).unwrap_or(0);
            self.stop_dictation("fin");
            let s = if n > 1 { "s" } else { "" };
            self.emit(json!({ "type": "say", "text": format!("Dictée terminée, {n} phrase{s} écrite{s}.") }));
            self.publish_status();
            return true;
        }
        if crate::re!(r"^corrige(?: moi)?(?: le texte| le document| les fautes| l orthographe| tout| mon texte| ce texte| la selection)?$").is_match(&t) {
            touch();
            let say = self.correct_document(!t.contains("selection")).await;
            self.emit(json!({ "type": "say", "text": say }));
            self.publish_status();
            return true;
        }
        if crate::re!(r"^(?:annule|efface|supprime) (?:ca|cela|la derniere phrase|le dernier passage|ce que je viens de dire)$").is_match(&t) {
            touch();
            self.desktop.undo().await;
            self.emit(json!({ "type": "say", "text": "Annulé." }));
            return true;
        }
        self.publish_status();
        if let Some(say) = self.dictate(clean).await {
            self.emit(json!({ "type": "say", "text": say }));
        }
        true
    }

    /// Macro personnelle : chaque étape locale s'exécute sans IA, dans l'ordre ; les autres partent à l'IA.
    pub async fn run_macro(self: &Rc<Self>, macro_: &J, _from_voice: bool) {
        let name = js::gs(macro_, "name");
        let steps = js::str_list(&macro_["steps"]);
        self.record("macro", json!({ "nom": name, "etapes": steps }));
        self.emit(json!({ "type": "say", "text": format!("Raccourci {name}.") }));
        for step in steps {
            match self.router.route(&step, false).filter(|i| !i.ignored) {
                Some(i) => {
                    self.run_intent(i, &step).await;
                }
                None => self.enqueue_ai(step, json!({})),
            }
        }
    }

    /// Envoi à une IA externe : tâche de fond ou premier plan, accusé de réception, file d'attente.
    pub fn enqueue_ai(self: &Rc<Self>, clean: String, meta: J) {
        let mut background = js::gb(&meta, "background") || js::gb(&meta, "isBackground");
        let mut text = clean;
        if let Some(m) = crate::re!(r"(?i)^(?:/fond|/bg|/background)\s+(.+)$").captures(&text) {
            background = true;
            text = m[1].trim().to_string();
        } else if crate::re!(r"(?i)\b(?:en\s+(?:fond|arri[eèé]re[- ]plan|t[aâ]che\s+de\s+fond))\b").is_match(&text) {
            background = true;
            let t = crate::re!(r"(?i)\b(?:en\s+(?:fond|arri[eèé]re[- ]plan|t[aâ]che\s+de\s+fond))\b").replace_all(&text, "");
            text = crate::re!(r"\s+").replace_all(&t, " ").trim().to_string();
        }
        // Priorité : 0 urgent, 1 interactif, 2 tâche de fond.
        let priority = meta["priority"].as_i64().unwrap_or_else(|| if crate::re!(r"(?i)\b(?:urgent|prioritaire|immediat|vite)\b").is_match(&text) { 0 } else if background { 2 } else { 1 });
        self.acknowledge(background, &text);
        let speaker = if meta["speaker"].is_object() {
            meta["speaker"].clone()
        } else if js::truthy(&meta["currentSpeaker"]) {
            json!({ "name": meta["currentSpeaker"] })
        } else {
            Value::Null
        };
        let item = QueuedTask { text, background, priority, queued_at: js::now_ms(), speaker };
        {
            let mut q = self.queue.borrow_mut();
            match q.iter().position(|i| i.priority > priority) {
                Some(i) => q.insert(i, item),
                None => q.push_back(item),
            }
        }
        self.next();
    }

    pub fn next(self: &Rc<Self>) {
        {
            let mut q = self.queue.borrow_mut();
            q.make_contiguous().sort_by(|a, b| a.priority.cmp(&b.priority).then(a.queued_at.cmp(&b.queued_at)));
        }
        loop {
            if self.running.borrow().len() >= self.max_concurrent {
                break;
            }
            let Some(item) = self.queue.borrow_mut().pop_front() else { break };
            self.execute(&item.text, item.background, json!({ "speaker": item.speaker, "priority": item.priority }));
        }
    }

    /// Accusé de réception immédiat et contextuel : la latence du modèle n'est plus un silence.
    pub fn acknowledge(&self, background: bool, request: &str) {
        let raw = request.to_lowercase();
        let pick = |l: &[&str]| l[js::rand_below(l.len())].to_string();
        if !background && !raw.is_empty() {
            let text = if crate::re!(r"(?i)\b(?:drive|google docs?|google sheets?)\b").is_match(&raw) {
                Some("Je regarde dans ton Google Drive.".to_string())
            } else if crate::re!(r"(?i)\b(?:fichiers?|dossiers?|documents?|t[ée]l[ée]chargements?)\b").is_match(&raw) && !crate::re!(r"(?i)\b(?:code|script|fonction|programme)\b").is_match(&raw) {
                Some("Je regarde tes fichiers.".to_string())
            } else if crate::re!(r"(?i)\b(?:code|script|fonction|git|commit|push|pull|terminal|powershell|bug|programme)\b").is_match(&raw) {
                Some(pick(&["J’analyse le code.", "Je vérifie les fichiers.", "Voyons le code."]))
            } else if crate::re!(r"(?i)\b(?:cherche|recherche|trouve|google|web|site|internet|en ligne|actualite|news)\b").is_match(&raw) {
                Some(pick(&["Je vérifie en ligne.", "Je regarde sur le web.", "Recherche en cours."]))
            } else if crate::re!(r"(?i)\b(?:calcule|combien|addition|soustraction|multiplication|division|pourcentage)\b").is_match(&raw) {
                Some("Je calcule cela.".to_string())
            } else {
                None
            };
            if let Some(t) = text {
                self.emit(json!({ "type": "say", "text": t, "ack": true }));
                return;
            }
        }
        let list = js::str_list(&phrases()[if background { "accuse_fond" } else { "accuse" }]);
        if list.is_empty() {
            return;
        }
        let mut i = js::rand_below(list.len());
        let last = self.last_ack.borrow().as_ref().map(|(t, _)| t.clone());
        if list.len() > 1 && last.as_deref() == Some(list[i].as_str()) {
            i = (i + 1) % list.len();
        }
        let text = list[i].clone();
        *self.last_ack.borrow_mut() = Some((text.clone(), js::now_ms()));
        self.emit(json!({ "type": "say", "text": text, "ack": true }));
    }
}
