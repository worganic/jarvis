// Vibe coding et perception de l'écran (orchestrator/vibe-coding.js) : pointer à l'écran (curseur) et
// demander à la voix une modification (« change ce bouton », « supprime cette carte »). Capture de l'écran
// avec le curseur, surlignage de l'élément dans le HUD, reformulation courte (« Bouton en vert ? ») selon
// la verbosité choisie, puis exécution par l'agent de code (démon persistant, sinon file de l'IA).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use serde_json::{json, Value};

use super::automations::base36;
use super::cli_daemon::DaemonTask;
use super::{OrchRef, Up};
use crate::js::{self, J};

fn norm(s: &str) -> String {
    js::fold_lower(s.trim())
}

pub fn vibe_trigger(t: &str) -> bool {
    crate::re!(r"(?i)(?:(?:regarde|modifie|change|ajuste|mets|remplace|supprime|retire|enleve|enlève|ajoute|corrige|ameliore|améliore)\s+(?:ce|cet|cette|ces|le|la|les|un|une|du|des|de\s+la|ici|la|mon|ma)?\s*(?:bouton|carte|texte|champ|zone|element|élément|bloc|composant|menu|titre|image|icone|icône|formulaire|div|navbar|footer|header|tableau|liste|couleur|taille|style|police|lien|page)|(?:en\s+vibe\s+coding|mode\s+vibe|vibe\s+code)|(?:change\s+ici|ajoute\s+ici|supprime\s+ici|regarde\s+ici|mets\s+ici|fais\s+ici|clique\s+ici))").is_match(t)
}

fn verbosity_trigger(t: &str) -> bool {
    crate::re!(r"(?i)(?:^(?:jarvis[, ]+)?(?:(?:passe (?:en|au)|active le|mets le)\s+)?mode\s+vibe\s+(ultra[\s-]court|court|standard|detaille|détaillé)(?:\b|$)|^(?:jarvis[, ]+)?(?:sois|parle)\s+(?:plus\s+court|plus\s+bref|bref|court)(?:\b|$)|^(?:jarvis[, ]+)?(?:sois|parle)\s+(?:plus\s+explicatif|plus\s+detaille|plus\s+détaillé|détaillé|detaille)(?:\b|$))").is_match(t)
}

fn confirm_yes() -> &'static regex::Regex {
    crate::re!(r"(?i)^(?:oui|vas[\s-]y|exact|exactement|d['’]accord|applique|valide|je\s+valide|lance|go|fais[\s-]le|yes|ok|parfait|(?:c['’]est\s+)?(?:bon|parfait|ca|ça|exact|super|bien))\b")
}
fn confirm_no() -> &'static regex::Regex {
    crate::re!(r"(?i)^(?:non|pas\s+ca|pas\s+ça|plutot|plutôt|en\s+fait|rectification|je\s+parlais\s+de|pas\s+celui[\s-]la|pas\s+celle[\s-]la)\b")
}
fn confirm_cancel() -> &'static regex::Regex {
    crate::re!(r"(?i)^(?:annule|annuler|stop|laisse\s+tomber|abandonne|quitte|oublie|arrete|arrête)\b")
}

const TARGET_KEYWORDS: &[&str] = &["bouton", "carte", "texte", "champ", "zone", "element", "bloc", "composant", "menu", "titre", "image", "icone", "formulaire", "navbar", "footer", "header", "tableau", "liste"];

pub struct VibeCoding {
    o: OrchRef,
    log: std::rc::Rc<dyn Fn(String)>,
    workspace: PathBuf,
    pub state: RefCell<String>,
    pub session: RefCell<Option<J>>,
    history: RefCell<Vec<J>>,
    counter: Cell<u64>,
    pub verbosite: RefCell<String>,
}

impl VibeCoding {
    pub fn new(o: OrchRef, log: std::rc::Rc<dyn Fn(String)>, workspace: PathBuf, verbosite: Option<String>) -> Self {
        Self { o, log, workspace, state: RefCell::new("idle".into()), session: RefCell::new(None), history: RefCell::new(vec![]), counter: Cell::new(0), verbosite: RefCell::new(verbosite.unwrap_or_else(|| "ultra-court".into())) }
    }

    /// Verbosité réglée dans la configuration (appelée au démarrage).
    pub fn load_verbosite(&self) {
        let v = js::gs(&self.o.up().config.get("vibeCoding"), "verbosite");
        if !v.is_empty() {
            *self.verbosite.borrow_mut() = v;
        }
    }

    pub fn set_verbosite(&self, mode: &str) -> &'static str {
        let clean = mode.to_lowercase();
        let v = match clean.trim() {
            "court" | "ultra-court" | "ultra court" => "ultra-court",
            "detaille" | "détaillé" | "long" => "detaille",
            _ => "standard",
        };
        *self.verbosite.borrow_mut() = v.into();
        self.o.up().config.update(&json!({ "vibeCoding": { "verbosite": v } }));
        match v {
            "ultra-court" => "Mode vibe ultra-court activé.",
            "detaille" => "Mode vibe détaillé activé.",
            _ => "Mode vibe standard activé.",
        }
    }

    pub fn handle_verbosity_switch(&self, text: &str) -> bool {
        let clean = norm(text);
        if !verbosity_trigger(&clean) {
            return false;
        }
        let target = if crate::re!(r"(?i)detaille|explicatif").is_match(&clean) {
            "detaille"
        } else if crate::re!(r"(?i)standard|normal").is_match(&clean) {
            "standard"
        } else {
            "ultra-court"
        };
        let msg = self.set_verbosite(target);
        self.o.up().emit(json!({ "type": "say", "text": msg }));
        true
    }

    pub fn is_vibe_utterance(&self, text: &str) -> bool {
        !text.is_empty() && vibe_trigger(text.trim())
    }

    pub fn has_pending_confirmation(&self) -> bool {
        *self.state.borrow() == "waiting_confirmation" && self.session.borrow().as_ref().map(|s| js::now_ms() - (js::gn_or(s, "startedAt", 0.0) as i64) < 45000).unwrap_or(false)
    }

    /// Capture rapide de l'écran avec le curseur et un rognage de 500 × 500 autour de lui.
    async fn capture_with_cursor(&self, crop: i64) -> J {
        let dir = self.workspace.join("captures").join("vibe");
        let _ = std::fs::create_dir_all(&dir);
        self.counter.set(self.counter.get() + 1);
        let target = dir.join(format!("vibe_{}_{}.png", js::now_ms(), self.counter.get()));
        let t2 = target.clone();
        let shot = tokio::task::spawn_blocking(move || jarvis_native::screenshot::screenshot(Some(&t2), "png", 90, crop as i32)).await.unwrap_or(Value::Null);
        if js::gb(&shot, "ok") || js::truthy(&shot["path"]) {
            return json!({
                "ok": true,
                "path": if js::truthy(&shot["path"]) { shot["path"].clone() } else { json!(target.to_string_lossy()) },
                "crop": shot["crop"],
                "width": js::gn_or(&shot, "width", 1920.0), "height": js::gn_or(&shot, "height", 1080.0),
                "cursor": if shot["cursor"].is_object() { shot["cursor"].clone() } else { json!({ "x": 0, "y": 0 }) },
                "window": if shot["window"].is_object() { shot["window"].clone() } else { json!({ "hwnd": 0, "title": "", "rect": { "x": 0, "y": 0, "width": 0, "height": 0 } }) },
                "duration_ms": js::gn_or(&shot, "duration_ms", 0.0)
            });
        }
        (self.log)(format!("[vibe-coding] Erreur capture native : {}", js::gs(&shot, "error")));
        json!({
            "ok": true, "path": target.to_string_lossy(), "crop": null, "width": 1920, "height": 1080, "cursor": { "x": 960, "y": 540 },
            "window": { "hwnd": 0, "title": "Bureau", "rect": { "x": 0, "y": 0, "width": 1920, "height": 1080 } }, "duration_ms": 0
        })
    }

    /// Élément ciblé et sa boîte estimée autour du curseur.
    pub fn detect_target_element(text: &str, cursor: &J) -> J {
        let clean = text.to_lowercase();
        let typ = TARGET_KEYWORDS.iter().find(|k| clean.contains(*k)).copied().unwrap_or("élément");
        let (w, h) = match typ {
            "bouton" => (160, 48),
            "texte" => (320, 40),
            "titre" => (400, 56),
            "carte" => (360, 260),
            "menu" => (220, 300),
            "champ" => (280, 44),
            "icone" => (48, 48),
            "image" => (300, 200),
            _ => (200, 80),
        };
        let x = (js::gn_or(cursor, "x", 0.0) as i64 - (w as f64 / 2.0).round() as i64).max(0);
        let y = (js::gn_or(cursor, "y", 0.0) as i64 - (h as f64 / 2.0).round() as i64).max(0);
        json!({ "type": typ, "label": js::capitalize(typ), "rect": { "x": x, "y": y, "width": w, "height": h } })
    }

    /// Action ultra-courte (2 à 4 mots).
    pub fn extract_short_action(user_text: &str, target: &J) -> String {
        let raw = user_text.to_lowercase();
        let raw = crate::re!(r"(?i)^(?:jarvis[, ]+)?").replace(&raw, "");
        let raw = crate::re!(r"(?i)^(?:regarde|s['’]il te plait|stp|merci)[, ]+").replace(&raw, "").trim().to_string();
        let typ = js::gs(target, "type");
        if crate::re!(r"(?i)^(?:supprime|retire|enleve|enlève|efface)").is_match(&raw) {
            let fem = ["carte", "zone", "image", "icone", "icône", "liste", "navbar", "div"].contains(&typ.as_str());
            return if fem { "supprimée".into() } else { "supprimé".into() };
        }
        if crate::re!(r"(?i)^(?:ajoute|cree|crée|mets un|mets une)").is_match(&raw) {
            if crate::re!(r"(?i)\bici\b").is_match(&raw) {
                return "ajouté ici".into();
            }
            if let Some(m) = crate::re!(r"(?i)^(?:ajoute|cree|crée|mets)\s+(?:un|une|du|des|de la)?\s*(.+)").captures(&raw) {
                let item = crate::re!(r"(?i)^(?:bouton|carte|texte|champ|zone|element|bloc)\s+(?:de\s+|pour\s+)?").replace(&m[1], "").trim().to_string();
                return if item.is_empty() { "ajouté".into() } else { item };
            }
            return "ajouté".into();
        }
        if crate::re!(r"(?i)^(?:agrandis|grossis|plus grand)").is_match(&raw) {
            return "agrandi".into();
        }
        if crate::re!(r"(?i)^(?:reduis|réduis|plus petit)").is_match(&raw) {
            return "réduit".into();
        }
        if let Some(m) = crate::re!(r"(?i)\b(en\s+[a-z0-9-]+(?:\s+[a-z0-9-]+){0,2}|au\s+centre|a\s+gauche|a\s+droite|en\s+gras|en\s+italique)").captures(&raw) {
            return m[1].trim().to_string();
        }
        let rest = crate::re!(r"(?i)^(?:modifie|change|ajuste|mets|remplace|passe|corrige|ameliore|améliore)\s+").replace(&raw, "");
        let rest = crate::re!(r"(?i)^(?:ce|cet|cette|ces|le|la|les|un|une|du|des|de la)\s+").replace(&rest, "");
        let rest = regex::Regex::new(&format!(r"(?i)^{}\s+", regex::escape(&typ))).map(|r| r.replace(&rest, "").into_owned()).unwrap_or_else(|_| rest.into_owned());
        let rest = crate::re!(r"(?i)^(?:de\s+couleur|en\s+couleur)\s+").replace(&rest, "").trim().to_string();
        if !rest.is_empty() {
            return rest.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
        }
        "modifié".into()
    }

    fn build_confirmation_question(&self, user_text: &str, target: &J, window_title: &str) -> String {
        let label = js::gs(target, "label");
        let v = self.verbosite.borrow().clone();
        if v == "ultra-court" {
            return format!("{label} {} ?", Self::extract_short_action(user_text, target));
        }
        let clean = crate::re!(r"(?i)^(?:jarvis[, ]+)?").replace(user_text, "");
        let clean = crate::re!(r"(?i)^(?:regarde|modifie|change|ajuste|mets|remplace|supprime|retire|enleve|enlève|ajoute)\s+").replace(&clean, "").trim().to_string();
        if v == "detaille" {
            let win = if window_title.is_empty() { "à l'écran".to_string() } else { format!("sur {window_title}") };
            return format!("J'ai ciblé {label} {win}. Tu veux que j'applique : « {clean} » ?");
        }
        format!("{label} ciblé. Tu veux {}, c'est bien ça ?", if clean.is_empty() { "cette modification".to_string() } else { clean })
    }

    /// Début d'une session de vibe coding.
    pub async fn start_session(&self, raw: &str, meta: J) -> J {
        *self.state.borrow_mut() = "aligning".into();
        let started = js::now_ms();
        self.counter.set(self.counter.get() + 1);
        let id = format!("vibe_{}_{}", base36(started as u64), self.counter.get());
        let capture = self.capture_with_cursor(500).await;
        let target = Self::detect_target_element(raw, &capture["cursor"]);
        let question = self.build_confirmation_question(raw, &target, &js::gs(&capture["window"], "title"));
        *self.session.borrow_mut() = Some(json!({ "id": id, "instruction": raw, "target": target, "capture": capture, "question": question, "startedAt": started, "meta": meta }));
        *self.state.borrow_mut() = "waiting_confirmation".into();
        let o = self.o.up();
        o.emit(json!({ "type": "vibe_highlight", "id": id, "rect": target["rect"], "label": target["label"], "ttl": 45000 }));
        o.emit(json!({ "type": "say", "text": question, "vibeId": id }));
        o.record("vibe_session_debut", json!({ "id": id, "instruction": raw, "cible": target["type"], "cursor": capture["cursor"] }));
        json!({ "ok": true, "id": id, "target": target, "question": question })
    }

    /// Réponse de confirmation de l'utilisateur : true si elle a été traitée ici.
    pub async fn handle_confirmation(&self, raw: &str) -> bool {
        if !self.has_pending_confirmation() {
            return false;
        }
        let clean = raw.trim();
        let o = self.o.up();
        let short = *self.verbosite.borrow() == "ultra-court";
        if confirm_cancel().is_match(clean) {
            self.clear_highlight();
            o.emit(json!({ "type": "say", "text": if short { "Annulé." } else { "Modification annulée." } }));
            self.clear_session("annulee");
            return true;
        }
        if confirm_yes().is_match(clean) {
            *self.state.borrow_mut() = "executing".into();
            if !short {
                o.emit(json!({ "type": "say", "text": "C'est parti, j'applique la modification." }));
            }
            let s = self.session.borrow().clone();
            if let Some(s) = s {
                self.execute_session(s).await;
            }
            return true;
        }
        if confirm_no().is_match(clean) {
            let refinement = confirm_no().replace(clean, "");
            let refinement = crate::re!(r"^[,;:\-\s]+").replace(&refinement, "");
            let refinement = crate::re!(r"(?i)^(?:plutot|plutôt|en\s+fait|rectification|je\s+parlais\s+de)\s+").replace(&refinement, "").trim().to_string();
            if !refinement.is_empty() {
                let q = if short { format!("{} ?", js::capitalize(&refinement)) } else { format!("Entendu. Tu veux {refinement}, je valide ?") };
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s["instruction"] = json!(format!("{} (Précision : {refinement})", js::gs(s, "instruction")));
                    s["question"] = json!(q);
                    s["startedAt"] = json!(js::now_ms());
                }
                o.emit(json!({ "type": "say", "text": q }));
            } else {
                o.emit(json!({ "type": "say", "text": if short { "Que changer ?" } else { "Que souhaites-tu modifier exactement sur cet élément ?" } }));
            }
            return true;
        }
        false
    }

    /// Exécution par l'agent de code.
    async fn execute_session(&self, s: J) {
        let cap = &s["capture"];
        let crop = js::gs(cap, "crop");
        let mut lines = vec![
            "[TÂCHE DE VIBE CODING TEMPS RÉEL]".to_string(),
            "L'utilisateur a pointé un élément à l'écran et demandé la modification suivante :".into(),
            format!("« {} »", js::gs(&s, "instruction")),
            String::new(),
            "Contexte visuel :".into(),
            format!("- Fenêtre cible : {}", if js::truthy(&cap["window"]["title"]) { js::gs(&cap["window"], "title") } else { "Inconnue".into() }),
            format!("- Position du curseur : X={}, Y={}", js::s(&cap["cursor"]["x"]), js::s(&cap["cursor"]["y"])),
            format!("- Type d'élément détecté : {}", js::gs(&s["target"], "label")),
            format!("- Capture d'écran complète : {}", js::gs(cap, "path")),
        ];
        if !crop.is_empty() {
            lines.push(format!("- Crop focalisé sous le curseur : {crop}"));
        }
        lines.extend([
            String::new(),
            "Consigne pour l'agent de code :".into(),
            "1. Identifie le fichier source correspondant à ce composant dans le projet.".into(),
            "2. Applique directement la modification demandée (CSS/HTML/JS/TSX) avec un minimum de changements nécessaires.".into(),
            "3. Sauvegarde le fichier pour déclencher le hot-reload.".into(),
        ]);
        let prompt = lines.join("\n");
        let o = self.o.up();
        o.record("vibe_execution", json!({ "id": s["id"], "instruction": s["instruction"], "cible": s["target"]["label"] }));
        if let Some(cfg) = o.daemon_config() {
            if o.cli_daemon.can_serve(&cfg.signature) {
                let (oc1, oc2) = (self.o.clone(), self.o.clone());
                let (s1, s2) = (s.clone(), s.clone());
                let log = self.log.clone();
                let ok = o.cli_daemon.submit(
                    &prompt,
                    DaemonTask {
                        on_event: Box::new(|_| {}),
                        on_finish: Box::new(move |is_error| {
                            js::spawn(async move { oc1.up().vibe_coding.on_execution_finished(s1, !is_error).await });
                        }),
                        on_error: Box::new(move |e| {
                            log(format!("[vibe-coding] Erreur agent : {e}"));
                            js::spawn(async move { oc2.up().vibe_coding.on_execution_finished(s2, false).await });
                        }),
                    },
                );
                if ok {
                    return;
                }
            }
        }
        o.enqueue_ai(prompt, json!({ "priority": 1 }));
        self.on_execution_finished(s, true).await;
    }

    async fn on_execution_finished(&self, s: J, success: bool) {
        *self.state.borrow_mut() = "verifying".into();
        self.clear_highlight();
        let o = self.o.up();
        let v = self.verbosite.borrow().clone();
        if success {
            let post = self.capture_with_cursor(500).await;
            o.emit(json!({ "type": "vibe_finished", "id": s["id"], "status": "success", "before": s["capture"]["path"], "after": post["path"] }));
            let msg = match v.as_str() {
                "ultra-court" => "Fait.",
                "standard" => "Modification appliquée et rechargée à l'écran.",
                _ => "Modification effectuée et rechargée sur ton écran. Regarde si le rendu te convient.",
            };
            o.emit(json!({ "type": "say", "text": msg }));
        } else {
            o.emit(json!({ "type": "say", "text": if v == "ultra-court" { "Échec." } else { "L'agent n'a pas pu finaliser la modification demandée." } }));
        }
        self.clear_session(if success { "succes" } else { "echec" });
    }

    fn clear_highlight(&self) {
        self.o.up().emit(json!({ "type": "vibe_clear" }));
    }

    fn clear_session(&self, result: &str) {
        if let Some(mut s) = self.session.borrow_mut().take() {
            s["result"] = json!(result);
            s["endedAt"] = json!(js::now_ms());
            let mut h = self.history.borrow_mut();
            h.push(s);
            if h.len() > 20 {
                h.remove(0);
            }
        }
        *self.state.borrow_mut() = "idle".into();
    }
}
