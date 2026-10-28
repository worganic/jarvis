// Noyau de Jarvis v4 (core/main.js de la v3) : processus autonome, lancé par la coquille Tauri (ou
// seul, pour les tests : jarvis-core). Il réunit :
//   - l'orchestrateur (routeur, sécurité, mémoire, journal scellé, connecteurs) ;
//   - le moteur vocal (jarvis-voice.exe), supervisé et relié ici (voice_link.rs) ;
//   - la passerelle WebSocket vers les fenêtres (gateway.rs) ;
//   - le service d'administration (liste blanche, fenêtre « admin » seulement) ;
//   - la mesure réelle de la consommation (metrics.rs).
//
// Dialogue avec la coquille :
//   - sortie standard : lignes « @@shell {json} » = commandes de fenêtre (ouvrir l'admin, widget…),
//     les autres lignes sont du journal ;
//   - entrée standard : lignes JSON de la coquille ; sa fermeture = arrêt du noyau (la coquille
//     est morte, rien ne doit lui survivre).

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use crate::code_watch::CodeWatch;
use crate::gateway::{Client, Gateway};
use crate::js::{self, Timer, J};
use crate::metrics::{Metrics, Roots};
use crate::module_api::{CoreAccess, Module, ModuleCtx};
use crate::modules::Registry;
use crate::orch::admin::AdminService;
use crate::orch::{Fut, OrchOptions, Orchestrator};
use crate::voice_link::VoiceLink;
use crate::voice_supervisor::VoiceSupervisor;

fn env_num(key: &str, default: u16) -> u16 {
    js::env(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn write(message: &str) {
    let text = message.trim_end();
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
    crate::logger::logger().info("noyau", text);
}

/// Commande de fenêtre pour la coquille.
fn shell(cmd: &str, payload: J) {
    let mut msg = json!({ "cmd": cmd });
    js::assign(&mut msg, payload);
    let mut out = std::io::stdout();
    let _ = writeln!(out, "@@shell {msg}");
    let _ = out.flush();
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    let _ = getrandom::getrandom(&mut b);
    hex::encode(b)
}

pub struct App {
    test: bool,
    data_dir: PathBuf,
    gateway: Rc<Gateway>,
    orch: RefCell<Option<Rc<Orchestrator>>>,
    supervisor: RefCell<Option<Rc<VoiceSupervisor>>>,
    voice: RefCell<Option<Rc<VoiceLink>>>,
    code_watch: RefCell<Option<Rc<CodeWatch>>>,
    backup: RefCell<Option<Rc<crate::backup::Backup>>>,
    /// Mises à jour d'un Jarvis installé (update.rs) ; inactif en développement.
    updater: RefCell<Option<Rc<crate::update::Updater>>>,
    admin: RefCell<Option<Rc<AdminService>>>,
    metrics: RefCell<Option<Rc<Metrics>>>,
    /// Parties noyau des modules liées au binaire (crates/jarvis-modules) et registre des fiches.
    modules: Vec<Rc<dyn Module>>,
    registry: Rc<Registry>,
    studio: crate::module_studio::Studio,
    started_modules: RefCell<Vec<Rc<dyn Module>>>,
    /// Services du cœur prêtés aux modules (ils n'en gardent qu'une référence faible).
    core_bridge: RefCell<Option<Rc<dyn CoreAccess>>>,
    metrics_timer: Timer,
    restarting: Cell<bool>,
    stopping: Cell<bool>,
    last_voice_config: RefCell<J>,
    done: RefCell<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl App {
    fn o(&self) -> Rc<Orchestrator> {
        self.orch.borrow().clone().expect("orchestrateur")
    }

    fn sup(&self) -> Rc<VoiceSupervisor> {
        self.supervisor.borrow().clone().expect("superviseur")
    }

    fn vl(&self) -> Rc<VoiceLink> {
        self.voice.borrow().clone().expect("liaison vocale")
    }

    /// Tout événement de l'orchestrateur : d'abord la voix et la coquille, puis les fenêtres.
    fn emit(self: &Rc<Self>, event: J) {
        let typ = js::gs(&event, "type");
        match typ.as_str() {
            "open_admin" => {
                shell("open_admin", json!({ "section": if js::truthy(&event["section"]) { event["section"].clone() } else { Value::Null } }));
                return;
            }
            "restart_jarvis" => {
                self.restart_jarvis("voix");
                return;
            }
            "autostart" => shell("autostart", json!({ "enable": js::truthy(&event["enable"]) })),
            "widget_mode" => shell("widget_mode", json!({ "on": js::truthy(&event["on"]) })),
            "desk_goto" => {
                shell("desk_goto", json!({}));
                return;
            }
            "desk_enable" => {
                let on = js::truthy(&event["on"]);
                let o = self.o();
                o.config.update(&json!({ "desk": { "enabled": on } }));
                o.record("reglages_modifies", json!({ "sections": ["desk"], "bureauJarvis": on, "origine": "voix" }));
                self.on_config_change(&["desk".to_string()], &o.config.all());
                return;
            }
            _ => {}
        }
        if let Some(v) = self.voice.borrow().clone() {
            v.on_orchestrator_event(&event);
        }
        self.gateway.broadcast(&event, None);
    }

    /// Code de Jarvis modifié : relance complète au clic ou à la voix.
    fn restart_jarvis(self: &Rc<Self>, origin: &str) -> J {
        if self.restarting.get() {
            return json!({ "ok": false, "reason": "relance déjà en cours" });
        }
        self.restarting.set(true);
        let st = self.code_watch.borrow().as_ref().map(|c| c.state()).unwrap_or_else(|| json!({ "files": [], "needsBuild": false }));
        let shell_pid: u32 = js::env("JARVIS_SHELL_PID").and_then(|p| p.parse().ok()).unwrap_or(0);
        let rebuild = js::gb(&st, "needsBuild") && shell_pid != 0;
        self.o().record("relance_jarvis", json!({ "origine": origin, "fichiers": st["files"], "recompilation": rebuild }));
        write(&format!("[noyau] relance complète demandée ({origin}){}", if rebuild { " : recompilation de Jarvis" } else { "" }));
        self.gateway.broadcast(&json!({ "type": "jarvis_restarting", "rebuild": rebuild }), None);
        let delay: u64 = if origin == "voix" { 2500 } else { 1200 }; // laisse finir « Je redémarre »
        if rebuild {
            // Jarvis entier est compilé : `jarvis-core relaunch` (sa propre console) attend la
            // fermeture de Jarvis, recompile et rouvre. Il tourne depuis une copie de l'exécutable,
            // pour que cargo puisse remplacer jarvis-core.exe.
            match crate::relaunch::spawn_console(shell_pid) {
                Ok(()) => {
                    js::set_timeout(delay, || shell("quit", json!({})));
                }
                Err(why) => {
                    write(&format!("[noyau] relance impossible : {why}"));
                    self.restarting.set(false);
                    if let Some(c) = self.code_watch.borrow().as_ref() {
                        self.gateway.broadcast(&c.state(), None);
                    }
                    self.emit(json!({ "type": "say", "text": "Je n'ai pas pu lancer la mise à jour. Relance-moi à la main." }));
                }
            }
        } else {
            // Le noyau s'arrête ; la coquille le relance aussitôt (avec le moteur vocal).
            let me = Rc::downgrade(self);
            js::set_timeout(delay, move || {
                if let Some(a) = me.upgrade() {
                    a.shutdown("relance demandée");
                }
            });
        }
        json!({ "ok": true, "rebuild": rebuild })
    }

    /// Résultat d'une relance avec recompilation (écrit par `jarvis-core relaunch`).
    fn report_relaunch(self: &Rc<Self>) {
        let file = self.data_dir.join("runtime").join("relaunch.json");
        let Some(res) = js::read_json(&file) else { return };
        let _ = std::fs::remove_file(&file);
        let fresh = js::parse_date_ms(&js::gs(&res, "at")).map(|t| js::now_ms() - t <= 10 * 60 * 1000).unwrap_or(false);
        if !js::truthy(&res["build"]) || !fresh {
            return;
        }
        self.o().record("relance_recompilation", res["build"].clone());
        if !js::gb(&res["build"], "ok") {
            let me = Rc::downgrade(self);
            js::set_timeout(4000, move || {
                if let Some(a) = me.upgrade() {
                    a.emit(json!({ "type": "say", "text": "La recompilation a échoué : j'ai redémarré avec la version précédente." }));
                }
            });
        }
    }

    fn set_metrics_log(&self, enabled: bool) {
        if self.test {
            return;
        }
        if !enabled {
            self.metrics_timer.clear();
            return;
        }
        if self.metrics_timer.active() {
            return;
        }
        let Some(m) = self.metrics.borrow().clone() else { return };
        m.refresh();
        self.metrics_timer.every(60000, move || {
            let s = m.collect();
            if js::gn(&s, "at").unwrap_or(0.0) > 0.0 {
                let rows = js::arr(&s["rows"]).iter().map(|r| format!("{} {} Mo/{:.1}%", js::gs(r, "label"), js::s(&r["mb"]), js::gn(r, "cpu").unwrap_or(0.0))).collect::<Vec<_>>().join(", ");
                crate::logger::logger().info("metrics", &format!("total {} Mo, CPU {:.1}% | {rows}", js::s(&s["mb"]), js::gn(&s, "cpu").unwrap_or(0.0)));
            }
        });
    }

    fn apply_performance(&self, perf: &J) -> bool {
        if !js::truthy(perf) {
            return false;
        }
        let sup = self.sup();
        let was_paused = sup.paused.get();
        sup.set_paused(!js::truthy(&perf["voiceListening"]));
        self.set_metrics_log(js::truthy(&perf["metricsLog"]));
        was_paused && !sup.paused.get()
    }

    fn on_config_change(self: &Rc<Self>, changed: &[String], cfg: &J) {
        let has = |k: &str| changed.iter().any(|c| c == k);
        let resumed = if has("performance") { self.apply_performance(&self.o().effective_performance()) } else { false };
        if has("voice") && !resumed {
            let prev = self.last_voice_config.borrow().clone();
            let next = cfg["voice"].clone();
            let live = ["minEndpointingDelay", "maxEndpointingDelay", "minInterruptionDuration", "allowInterruptions"];
            let flat = |v: &J| {
                let mut m = if v["livekit"].is_object() { v["livekit"].clone() } else { json!({}) };
                if v["engine"].is_object() {
                    js::assign(&mut m, v["engine"].clone());
                }
                m
            };
            let (a, b) = (flat(&prev), flat(&next));
            let mut keys: Vec<String> = a.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
            for k in b.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            let diff: Vec<String> = keys.into_iter().filter(|k| a[k] != b[k]).collect();
            if !diff.is_empty() && diff.iter().all(|k| live.contains(&k.as_str())) {
                let mut m = json!({ "t": "tune" });
                for k in &diff {
                    m[k] = b[k].clone();
                }
                self.vl().send(m);
            } else if !diff.is_empty() {
                self.sup().restart();
            }
        }
        if has("voice") {
            *self.last_voice_config.borrow_mut() = cfg["voice"].clone();
        }
        if has("widget") {
            shell("widget_config", json!({ "widget": cfg["widget"] }));
            let mut ev = json!({ "type": "widget_config" });
            js::assign(&mut ev, cfg["widget"].clone());
            self.gateway.broadcast(&ev, None);
        }
        if has("desk") {
            shell("desk_config", json!({ "desk": cfg["desk"] }));
            let mut ev = json!({ "type": "desk_config" });
            js::assign(&mut ev, cfg["desk"].clone());
            self.gateway.broadcast(&ev, None);
        }
    }

    fn shutdown(self: &Rc<Self>, reason: &str) {
        if self.stopping.replace(true) {
            return;
        }
        write(&format!("[noyau] arrêt ({reason})"));
        if let Some(c) = self.code_watch.borrow().as_ref() {
            c.stop();
        }
        if let Some(v) = self.voice.borrow().as_ref() {
            v.stop();
        }
        for m in self.started_modules.borrow_mut().drain(..) {
            m.stop();
        }
        if let Some(s) = self.supervisor.borrow().as_ref() {
            s.stop();
        }
        if let Some(o) = self.orch.borrow().as_ref() {
            o.stop();
        }
        self.gateway.stop();
        let me = Rc::downgrade(self);
        js::set_timeout(300, move || {
            if let Some(a) = me.upgrade() {
                if let Some(tx) = a.done.borrow_mut().take() {
                    let _ = tx.send(());
                }
            }
        });
    }

    /// Données reprises de la v3 (tools : jarvis-core migrate) : la première entrée du journal v4
    /// cite l'empreinte finale du journal v3, qui reste intact de son côté.
    fn record_migration(&self) {
        let file = self.data_dir.join("migration-v3.json");
        let Some(mut report) = js::read_json(&file) else { return };
        if js::truthy(&report["recorded"]) {
            return;
        }
        self.o().record(
            "migration_v3",
            json!({
                "depuis": report["from"], "fichiers": report["copied"],
                "journalV3": if js::truthy(&report["v3Journal"]) { report["v3Journal"].clone() } else { Value::Null },
                "secretsARessaisir": if report["secretsToReenter"].is_array() { report["secretsToReenter"].clone() } else { json!([]) }
            }),
        );
        report["recorded"] = json!(true);
        let _ = js::write_json(&file, &report);
    }

    fn on_connect(self: &Rc<Self>, ws: &Rc<Client>) {
        let o = self.o();
        if ws.role == "admin" {
            o.record("administration_ouverte", json!({}));
            return;
        }
        let g = &self.gateway;
        g.send(ws, &self.vl().snapshot());
        if let Some(c) = self.code_watch.borrow().as_ref() {
            g.send(ws, &c.state());
        }
        // Deux pages HUD peuvent coexister (fenêtre classique et Bureau JARVIS).
        let mut desk = json!({ "type": "desk_config" });
        js::assign(&mut desk, o.config.get("desk"));
        g.send(ws, &desk);
        if ws.role == "widget" {
            let mut w = json!({ "type": "widget_config" });
            js::assign(&mut w, o.config.get("widget"));
            g.send(ws, &w);
            return;
        }
        // Cards que ce noyau connaît : après une relance du noyau (fenêtres restées ouvertes), le HUD
        // retire celles de l'ancien noyau.
        g.send(ws, &json!({ "type": "cards_sync", "keys": o.cards.keys() }));
        if let Some(card) = o.desktop_card_replay() {
            g.send(ws, &card);
        }
        o.publish_memory();
        o.publish_journal();
        o.publish_status();
        o.publish_performance();
        o.publish_vocabulary(true);
        o.publish_audio_device();
    }
}

impl App {
    /// Démarre la partie noyau des modules actifs (fiche présente et non désactivée).
    fn start_modules(self: &Rc<Self>) {
        let core: Rc<dyn CoreAccess> = Rc::new(CoreBridge(Rc::downgrade(self)));
        let weak = Rc::downgrade(&core);
        *self.core_bridge.borrow_mut() = Some(core);
        for m in &self.modules {
            let id = m.id();
            if !self.registry.is_enabled(id) {
                write(&format!("[modules] {id} : désactivé ou sans fiche, non démarré"));
                continue;
            }
            let ctx = ModuleCtx::new(id, self.gateway.clone(), weak.clone(), Rc::new(|m: String| write(&m)), self.data_dir.join("modules").join(id));
            m.start(ctx);
            self.started_modules.borrow_mut().push(m.clone());
        }
    }
}

/// Services du cœur prêtés aux modules (module_api.rs › CoreAccess).
struct CoreBridge(std::rc::Weak<App>);

impl CoreAccess for CoreBridge {
    fn emit(&self, ev: J) {
        if let Some(a) = self.0.upgrade() {
            a.emit(ev);
        }
    }
    fn record(&self, kind: &str, data: J) {
        if let Some(a) = self.0.upgrade() {
            a.o().record(kind, data);
        }
    }
    fn jarvis_volume(&self) -> i64 {
        self.0.upgrade().map(|a| a.vl().jarvis_volume.get()).unwrap_or(0)
    }
    fn audio_device_changed(&self) -> Fut<()> {
        let w = self.0.clone();
        Box::pin(async move {
            let dev = crate::orch::system::get_audio_device().await;
            let Some(app) = w.upgrade() else { return };
            let vl = app.vl();
            if js::gb(&dev, "ok") {
                *vl.audio_mode.borrow_mut() = Some(js::gs(&dev, "mode"));
            }
            vl.send(json!({ "t": "reopen_audio" }));
            if js::gb(&dev, "ok") {
                let mut ev = json!({ "type": "audio_device" });
                js::assign(&mut ev, dev);
                app.emit(ev);
            }
        })
    }
}

fn ok<T: Into<J>>(v: T) -> Result<J, String> {
    Ok(v.into())
}

fn arg(a: &[J], i: usize) -> J {
    a.get(i).cloned().unwrap_or(Value::Null)
}

/// Réponse d'un bouton de card : sa phrase éventuelle est dite.
fn card_say(o: &Orchestrator, r: J) -> J {
    if js::truthy(&r["say"]) {
        o.emit(json!({ "type": "say", "text": r["say"] }));
    }
    r
}

fn register_methods(app: &Rc<App>) {
    let g = app.gateway.clone();
    macro_rules! reg {
        ($name:expr, $roles:expr, |$a:ident, $c:ident, $app:ident| $body:expr) => {{
            let w = Rc::downgrade(app);
            g.register($name, $roles, move |$a: Vec<J>, $c: Rc<Client>| -> Fut<Result<J, String>> {
                let Some($app) = w.upgrade() else { return Box::pin(async { Err("noyau arrêté".to_string()) }) };
                let _ = (&$a, &$c, &$app);
                Box::pin(async move { $body })
            });
        }};
    }
    const HW: &[&str] = &["hud", "widget"];
    const HUD: &[&str] = &["hud"];

    reg!("utterance", HW, |a, c, app| {
        let mut meta = if arg(&a, 1).is_object() { arg(&a, 1) } else { json!({}) };
        meta["source"] = json!("typed");
        app.o().submit(js::s(&arg(&a, 0)), meta).await;
        ok(Value::Null)
    });
    reg!("speculativeUtterance", HW, |a, c, app| {
        app.o().on_speculative_utterance(&js::s(&arg(&a, 0)), &arg(&a, 1));
        ok(Value::Null)
    });
    reg!("approve", HW, |a, c, app| {
        app.o().approve(&js::s(&arg(&a, 0)));
        ok(Value::Null)
    });
    // Card fermée par l'utilisateur : la tâche est arrêtée (processus IA tué, validation refusée).
    reg!("cancelTask", HW, |a, c, app| {
        let o = app.o();
        let id = js::s(&arg(&a, 0));
        if !o.is_running(&id) {
            return ok(json!({ "ok": false, "reason": "tâche déjà terminée" }));
        }
        o.record("tache_fermee", json!({ "tache": id, "origine": "card" }));
        ok(json!({ "ok": o.cancel(&id) }))
    });
    reg!("deny", HW, |a, c, app| {
        app.o().deny(&js::s(&arg(&a, 0)));
        ok(Value::Null)
    });
    // Clavier numérique du code de validation : chiffres vérifiés par le noyau.
    reg!("codeDigits", HW, |a, c, app| {
        let id = js::s(&arg(&a, 0));
        let digits = js::slice_chars(&js::s(&arg(&a, 1)), 4);
        ok(app.o().code_entry.digits(&digits, (!id.is_empty()).then_some(id.as_str()), "clic"))
    });
    reg!("codeBack", HW, |a, c, app| {
        let id = js::s(&arg(&a, 0));
        ok(app.o().code_entry.back((!id.is_empty()).then_some(id.as_str())))
    });
    reg!("cardClosed", HW, |a, c, app| {
        let o = app.o();
        let key = js::s(&arg(&a, 0));
        if key == "note" {
            o.notes.card_closed();
        }
        ok(o.cards.closed(&key))
    });
    // Bureau : boutons des cards « Bureau » / « Chrome ».
    reg!("desktopAction", HW, |a, c, app| {
        let action = js::s(&arg(&a, 1));
        // Emplacements de la card Bureau : « slot:2 », « layout_save:3 »…
        if crate::re!(r"^(?:slot|slot_save|slot_clear|layout|layout_save|layout_clear):[1-4]$").is_match(&action) {
            let o = app.o();
            let r = o.window_slot_action(js::num(&arg(&a, 0)).unwrap_or(0.0) as i64, &action).await;
            return ok(card_say(&o, r));
        }
        // Applications en arrière-plan : masquer / remettre / afficher les masquées.
        if action == "bg_toggle" || action.starts_with("bg_hide:") || action.starts_with("bg_show:") {
            let o = app.o();
            match action.split_once(':') {
                Some((verb, name)) => o.bg_hidden.set(name, verb == "bg_hide"),
                None => o.bg_hidden.showing.set(!o.bg_hidden.showing.get()),
            }
            o.desktop_card_sig.borrow_mut().clear();
            o.refresh_desktop_card();
            return ok(json!({ "ok": true }));
        }
        if !["afficher", "centrer", "deplacer", "reduire", "agrandir", "restaurer", "fermer"].contains(&action.as_str()) {
            return ok(json!({ "ok": false }));
        }
        let o = app.o();
        let r = o.desktop_action(js::num(&arg(&a, 0)).unwrap_or(0.0) as i64, &action).await;
        ok(card_say(&o, r))
    });
    // Card « Devant » : retirer temporairement/définitivement, restaurer, sous-menu.
    reg!("devantAction", HW, |a, c, app| ok(app.o().devant_action(&js::s(&arg(&a, 0)), &js::s(&arg(&a, 1)))));
    // Card « Éléments de <appli> » : cliquer sur un bouton de l'application (UI Automation).
    reg!("uiClick", HUD, |a, c, app| {
        let o = app.o();
        let name = if arg(&a, 2).is_string() { js::s(&arg(&a, 2)) } else { String::new() };
        let r = o.ui_click_action(js::num(&arg(&a, 0)).unwrap_or(0.0) as i64, js::num(&arg(&a, 1)).unwrap_or(0.0) as i64, &name).await;
        ok(card_say(&o, r))
    });
    // Card Chrome : ligne d'onglet (aller dessus, recharger, fermer).
    reg!("browserTab", HUD, |a, c, app| {
        let o = app.o();
        let r = o.browser_tab_action(js::num(&arg(&a, 0)).unwrap_or(0.0) as i64, &js::s(&arg(&a, 1))).await;
        ok(card_say(&o, r))
    });
    // Boutons des cards de liste (rappels, raccourcis, fichiers, notes, applis, voix, agenda, projets, produits).
    reg!("listAction", HUD, |a, c, app| {
        let kind = js::s(&arg(&a, 0));
        if !["rappel", "raccourci", "fichier", "note", "appli", "voix", "agenda", "projet", "produit"].contains(&kind.as_str()) || !arg(&a, 2).is_string() {
            return ok(json!({ "ok": false }));
        }
        let o = app.o();
        let r = o.list_cards.action(&kind, &arg(&a, 1), &js::s(&arg(&a, 2))).await;
        ok(card_say(&o, r))
    });
    // Listes à fiches (vinyles, Lego…) : la card « Liste » et ses formulaires. Même service que la voix et la page d'aide.
    reg!("collections", HW, |a, c, app| {
        let op = js::s(&arg(&a, 0));
        let o = app.o();
        let r = o.collections.dispatch(&op, &arg(&a, 1), "card").await;
        ok(card_say(&o, r))
    });
    // Card Amis et Administration › Amis : Jarvis Réseau (orch/reseau.rs).
    reg!("reseau", &["hud", "widget", "admin"], |a, c, app| {
        let o = app.o();
        let r = o.reseau.clone();
        ok(r.dispatch(&js::s(&arg(&a, 0)), &arg(&a, 1)).await)
    });
    // Card Diction : les dernières phrases dites, mots cliquables, écoute et correction de la prononciation.
    reg!("diction", HUD, |a, c, app| {
        let o = app.o();
        ok(crate::orch::pronunciation::diction_op(&o, &js::s(&arg(&a, 0)), &arg(&a, 1)))
    });
    // Card Actions (card 5) : nombre d'éléments de chaque card, par module (affiché à droite de sa ligne).
    reg!("compteurs", HUD, |a, c, app| {
        let o = app.o();
        ok(json!({
            "listes": o.collections.count() + o.smart_lists.count(), "notes": o.notes.list().len(), "projets": o.projects.list().len(),
            "reunions": o.meetings.list().len(), "savoir": o.savoir.list().len(), "agents": o.agents.count()
        }))
    });
    // Card Actions : ouvre la card Checkup (scanner du cœur) sans passer par la voix ni l'IA.
    reg!("checkup", HUD, |a, c, app| {
        let o = app.o();
        o.show_checkup_card();
        ok(json!({ "ok": true }))
    });
    // Jarvis Nomade : la card (état de la synchronisation, dépôts faits depuis le site).
    reg!("nomade", HUD, |a, c, app| {
        let o = app.o();
        ok(match js::s(&arg(&a, 0)).as_str() {
            "etat" => o.nomade.state(),
            "synchroniser" => o.nomade.sync_now().await,
            "depot" => o.nomade.depot(arg(&a, 1).as_i64().unwrap_or(0), &js::s(&arg(&a, 2))).await,
            "ecart" => o.nomade.conflict(&js::s(&arg(&a, 1)), arg(&a, 2).as_i64().unwrap_or(0), &js::s(&arg(&a, 3))),
            _ => json!({ "ok": false, "error": "opération inconnue" }),
        })
    });
    // Compagnon de bureau (tamagotchi) : état, réglages, veille, repas (extension seulement), caresse, nouvel œuf.
    reg!("compagnon", HW, |a, c, app| {
        let o = app.o();
        let p = arg(&a, 1);
        ok(match js::s(&arg(&a, 0)).as_str() {
            "" | "etat" => o.companion.view(),
            "ouvrir" => {
                app.gateway.broadcast(&json!({ "type": "companion_open" }), None);
                json!({ "ok": true })
            }
            "regler" => o.companion.set_config(&p),
            "veille" => o.companion.set_asleep(js::gb(&p, "on")),
            "nourrir" => o.companion.feed(&js::gs(&p, "ext")),
            "caresse" => o.companion.caress(),
            "oeuf" => o.companion.restart(),
            _ => json!({ "ok": false, "error": "opération inconnue" }),
        })
    });
    // Module « Agents » : création depuis une demande texte, liste, modification, exécution et suppression.
    reg!("agents", HW, |a, c, app| {
        let o = app.o();
        let op = js::s(&arg(&a, 0));
        let p = arg(&a, 1);
        ok(match op.as_str() {
            "" | "etat" => o.agents.state(),
            "ouvrir" => {
                app.gateway.broadcast(&json!({ "type": "agents_open" }), None);
                json!({ "ok": true })
            }
            "analyser" => {
                let prompt = js::gs(&p, "prompt");
                json!({ "ok": true, "draft": crate::orch::agents::parse_prompt_local(&prompt) })
            }
            "creer" => o.agents.create_from_prompt(&js::gs(&p, "prompt"), &p).await,
            "modifier" => o.agents.update(&js::gs(&p, "id"), &p["patch"]),
            "modifier_prompt" => o.agents.update_from_prompt(&js::gs(&p, "id"), &js::gs(&p, "prompt")).await,
            "basculer" => o.agents.toggle(&js::gs(&p, "id"), p["actif"].as_bool()),
            "executer" => o.agents.execute_agent(&js::gs(&p, "id"), "Exécution manuelle").await,
            "supprimer" => o.agents.delete(&js::gs(&p, "id")),
            "effacer_historique" => o.agents.clear_history(&js::gs(&p, "id")),
            _ => json!({ "ok": false, "error": "opération inconnue" }),
        })
    });
    // Dossiers « Savoir » : la card Data (liste, fiche, approfondir, ajouter un lien, supprimer). Même service que la voix.
    reg!("savoir", HW, |a, c, app| {
        let op = js::s(&arg(&a, 0));
        let o = app.o();
        let r = o.savoir.dispatch(&op, &arg(&a, 1)).await;
        ok(card_say(&o, r))
    });
    // Mails : clic sur un mail d'une card → card de lecture ; boutons de la card de lecture.
    reg!("mailOpen", HUD, |a, c, app| {
        let o = app.o();
        let r = o.mails.open(&arg(&a, 0), false).await;
        write(&format!(
            "[mails] clic sur un mail ({}) : {}",
            arg(&a, 0),
            if js::gb(&r, "ok") { "card de lecture ouverte".to_string() } else { format!("échec — {}", js::gs(&r, "say")) }
        ));
        ok(card_say(&o, r))
    });
    reg!("mailAction", HUD, |a, c, app| {
        let action = js::s(&arg(&a, 1));
        if !["lire", "resumer", "repondre", "exclure", "gmail", "chrome"].contains(&action.as_str()) {
            return ok(json!({ "ok": false }));
        }
        let o = app.o();
        let r = o.mails.act(&arg(&a, 0), &action).await;
        ok(card_say(&o, r))
    });
    // Card de prise de notes : texte modifié à la main, boutons.
    reg!("noteEdit", HUD, |a, c, app| {
        let p = arg(&a, 1);
        let mut patch = json!({});
        for k in ["title", "body"] {
            if p[k].is_string() {
                patch[k] = p[k].clone();
            }
        }
        ok(app.o().notes.edit(&js::s(&arg(&a, 0)), &patch))
    });
    reg!("noteAction", HUD, |a, c, app| {
        let p = arg(&a, 2);
        let mut payload = json!({});
        for k in ["title", "body"] {
            if p[k].is_string() {
                payload[k] = p[k].clone();
            }
        }
        let o = app.o();
        ok(o.notes.action(&js::s(&arg(&a, 0)), &js::s(&arg(&a, 1)), &payload).await)
    });
    // Card de réunion et bannière Google Meet.
    reg!("meetingAction", HUD, |a, c, app| {
        let action = js::s(&arg(&a, 1));
        if !["start", "start_meet", "dismiss_offer", "stop", "title", "name_voice", "summary", "gdoc", "save", "list", "open"].contains(&action.as_str()) {
            return ok(json!({ "ok": false }));
        }
        let p = arg(&a, 2);
        let mut payload = json!({});
        let lim = [("code", 20usize), ("title", usize::MAX), ("voice", 10), ("name", 60), ("file", usize::MAX)];
        for (k, max) in lim {
            if p[k].is_string() {
                let s = js::s(&p[k]);
                payload[k] = json!(if max == usize::MAX { s } else { js::slice_chars(&s, max) });
            }
        }
        let o = app.o();
        ok(o.meetings.action(&js::s(&arg(&a, 0)), &action, &payload).await)
    });
    reg!("cardOpen", HW, |a, c, app| {
        let o = app.o();
        let r = o.cards.open(js::num(&arg(&a, 0)).unwrap_or(0.0) as i64).await;
        ok(card_say(&o, r))
    });
    reg!("cardClose", HW, |a, c, app| {
        let o = app.o();
        let r = o.cards.close(js::num(&arg(&a, 0)).unwrap_or(0.0) as i64, true).await;
        ok(card_say(&o, r))
    });
    reg!("cardAppOpen", HW, |a, c, app| {
        let o = app.o();
        let r = o.cards.open_app(&js::s(&arg(&a, 0))).await;
        ok(card_say(&o, r))
    });
    reg!("cardAppClose", HW, |a, c, app| {
        let o = app.o();
        let r = o.cards.close_app(&js::s(&arg(&a, 0))).await;
        ok(card_say(&o, r))
    });
    // Boutons de la card d'un terminal ou d'un Claude : « 🎙 Dicter ici », « ✋ Jarvis reprend la main ».
    reg!("cardAppAction", HUD, |a, c, app| {
        let action = js::s(&arg(&a, 1));
        if !(["dicter", "dictee", "reprendre", "confirm:oui", "confirm:non"].contains(&action.as_str()) || crate::re!(r"^act:[a-z0-9_-]{1,40}$").is_match(&action)) {
            return ok(json!({ "ok": false }));
        }
        let o = app.o();
        let r = o.cards.app_action(&js::s(&arg(&a, 0)), &action).await;
        ok(card_say(&o, r))
    });
    // Vision : image de la webcam prise par le HUD à la demande de Jarvis.
    reg!("cameraFrame", HUD, |a, c, app| ok(app.o().vision.frame(&js::s(&arg(&a, 0)), &js::s(&arg(&a, 1)))));
    reg!("cameraError", HUD, |a, c, app| ok(app.o().vision.fail(&js::s(&arg(&a, 0)), &js::s(&arg(&a, 1)))));
    // Présence : petite image RVB envoyée chaque seconde par le HUD quand le réglage est actif (rien n'est gardé).
    reg!("presenceFrame", HUD, |a, c, app| {
        let o = app.o();
        ok(o.presence.frame(&js::s(&arg(&a, 0))).await)
    });
    reg!("presenceConfig", HUD, |a, c, app| ok(app.o().presence.hud_config()));
    reg!("stopAll", HW, |a, c, app| ok(app.o().stop_all(if js::s(&arg(&a, 0)) == "echap" { "echap" } else { "bouton" })));
    reg!("restartVoice", HW, |a, c, app| {
        app.sup().restart();
        ok(Value::Null)
    });
    reg!("restartJarvis", &["hud", "widget", "admin"], |a, c, app| ok(app.restart_jarvis("clic")));
    // Mises à jour d'un Jarvis installé (update.rs).
    reg!("updateState", &["hud", "widget", "admin"], |a, c, app| ok(app.updater.borrow().as_ref().map(|u| u.state()).unwrap_or(json!({ "installe": false }))));
    reg!("updateCheck", &["hud", "admin"], |a, c, app| {
        let u = app.updater.borrow().clone();
        match u {
            Some(u) => ok(u.check().await),
            None => ok(json!({ "ok": false, "error": "mises à jour indisponibles" })),
        }
    });
    reg!("updatePrepare", &["hud", "admin"], |a, c, app| {
        let u = app.updater.borrow().clone();
        let tag = js::s(&arg(&a, 0));
        ok(match u {
            Some(u) => {
                let tag = if tag.is_empty() { js::gs(&u.state(), "disponible") } else { tag };
                if tag.is_empty() { json!({ "ok": false, "error": "aucune nouvelle version" }) } else { u.prepare(&tag) }
            }
            None => json!({ "ok": false, "error": "mises à jour indisponibles" }),
        })
    });
    // Installe la version préparée : le script attend la fermeture de Jarvis, qui se ferme aussitôt.
    reg!("updateApply", &["hud", "admin"], |a, c, app| {
        let r = app.updater.borrow().as_ref().map(|u| u.apply()).unwrap_or(json!({ "ok": false, "error": "mises à jour indisponibles" }));
        if js::gb(&r, "ok") {
            app.o().record("mise_a_jour_installee", json!({ "version": crate::update::ready().map(|p| p["tag"].clone()) }));
            js::set_timeout(1500, || shell("quit", json!({ "update": true })));
        }
        ok(r)
    });
    reg!("updateModule", &["admin"], |a, c, app| {
        let add = js::s(&arg(&a, 0)) == "ajouter";
        let id = js::s(&arg(&a, 1));
        let r = app.updater.borrow().as_ref().map(|u| u.module(add, &id)).unwrap_or(json!({ "ok": false, "error": "mises à jour indisponibles" }));
        if js::gb(&r, "ok") {
            app.o().record("module_installation", json!({ "action": if add { "ajout" } else { "retrait" }, "module": id }));
        }
        ok(r)
    });
    // Numéro de version affiché par le HUD ; un clic dessus ouvre l'historique (ui/changelog.html,
    // servi par la passerelle) dans le navigateur.
    reg!("appVersion", &["hud", "widget", "admin"], |a, c, app| ok(crate::changelog::info()));
    reg!("openChangelog", &["hud", "widget", "admin"], |a, c, app| {
        let url = format!("http://127.0.0.1:{}/ui/changelog.html", app.gateway.port);
        ok(crate::orch::system::ouvrir_url(&json!({ "url": url })).await.unwrap_or_else(|e| json!({ "ok": false, "error": e })))
    });
    // Bouton « ? AIDE » du HUD : la page d'aide complète (ui/aide.html) dans le navigateur.
    reg!("openHelp", &["hud", "widget", "admin"], |a, c, app| {
        // Le jeton de l'aide voyage dans le fragment de l'adresse : le navigateur ne l'envoie jamais au serveur.
        let url = format!("http://127.0.0.1:{}/ui/aide.html#j={}", app.gateway.port, app.gateway.aide_token());
        ok(crate::orch::system::ouvrir_url(&json!({ "url": url })).await.unwrap_or_else(|e| json!({ "ok": false, "error": e })))
    });
    // Bouton 📷 de la card Bureau : l'outil de capture de Windows (curseur de sélection de la zone, copie dans le presse-papier).
    reg!("screenClip", HW, |a, c, app| ok(json!({ "ok": crate::proc::detached("cmd.exe", &["/c", "start", "", "ms-screenclip:"]) })));
    reg!("codeState", HW,|a, c, app| ok(app.code_watch.borrow().as_ref().map(|c| c.state()).unwrap_or(Value::Null)));
    reg!("ptt", HUD, |a, c, app| {
        app.vl().set_ptt(js::truthy(&arg(&a, 0)));
        ok(Value::Null)
    });
    reg!("voiceStop", HW, |a, c, app| {
        app.vl().send(json!({ "t": "stop" }));
        ok(Value::Null)
    });
    // Bouton « son » du HUD : voix de Jarvis coupée / rétablie.
    reg!("voiceSilence", HW, |a, c, app| {
        let on = arg(&a, 0).as_bool();
        let o = app.o();
        ok(o.toggle_silence(on).await)
    });
    reg!("voiceMute", HUD, |a, c, app| {
        let vl = app.vl();
        let on = arg(&a, 0).as_bool().unwrap_or(!vl.mic_muted.get());
        ok(vl.set_mic_muted(on))
    });
    reg!("voiceState", HW, |a, c, app| {
        let mut st = app.vl().snapshot();
        if let Some(m) = st.as_object_mut() {
            m.remove("type");
        }
        ok(st)
    });
    reg!("mediaCommand", HW, |a, c, app| {
        let cmd = js::s(&arg(&a, 0));
        if !["play", "pause", "stop", "toggle", "next", "previous", "status"].contains(&cmd.as_str()) {
            return ok(Value::Null);
        }
        // Widget affichant une application externe (Spotify) : elle seule est pilotée.
        let app_name = if arg(&a, 1).is_string() { js::s(&arg(&a, 1)) } else { String::new() };
        let target = crate::re!(r"(?i)^[a-z0-9 ._!-]{1,80}$").is_match(&app_name) && !crate::re!(r"(?i)^jarvis").is_match(&app_name);
        let o = app.o();
        if target {
            return ok(o.media_action(json!({ "action": cmd, "app": app_name })).await);
        }
        app.gateway.broadcast(&json!({ "type": "music_control", "action": cmd }), None);
        ok(o.media_action(json!({ "action": cmd })).await)
    });
    reg!("widgetState", HW, |a, c, app| {
        if js::s(&arg(&a, 0)) == "music" {
            app.o().media.set_keep_alive(js::truthy(&arg(&a, 1)));
        }
        ok(Value::Null)
    });
    reg!("sendMusicState", HW, |a, c, app| {
        let state = arg(&a, 0);
        let o = app.o();
        *o.internal_music_state.borrow_mut() = state.clone();
        let mut ev = json!({ "type": "media" });
        js::assign(&mut ev, state);
        o.emit(ev);
        ok(Value::Null)
    });
    reg!("getLocalMusic", HW, |a, c, app| {
        let mut tracks = vec![];
        let home = js::env("USERPROFILE").map(PathBuf::from).unwrap_or_default();
        for dir in [app.data_dir.join("music"), home.join("Music")] {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            files.sort();
            for f in files {
                let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                if !["mp3", "wav", "flac", "ogg", "m4a"].contains(&ext.as_str()) {
                    continue;
                }
                let base = f.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                let parts: Vec<&str> = base.split(" - ").collect();
                let (title, artist) = if parts.len() > 1 { (parts[1..].join(" - ").trim().to_string(), parts[0].trim().to_string()) } else { (base.clone(), "Fichier Local".to_string()) };
                tracks.push(json!({
                    "id": format!("local-{}", tracks.len() + 1), "title": title, "artist": artist, "album": "Musique Locale",
                    // Lu par la coquille (protocole asset de Tauri), jamais par une URL file:// arbitraire.
                    "path": f.display().to_string(), "isStream": false
                }));
            }
        }
        ok(J::Array(tracks))
    });
    reg!("getErrors", HW, |a, c, app| ok(J::Array(crate::logger::logger().recent_errors(25))));
    reg!("clearErrors", HW, |a, c, app| ok(crate::logger::logger().clear_errors()));
    reg!("getAudioDevice", HW, |a, c, app| ok(crate::orch::system::get_audio_device().await));
    reg!("toggleAudioDevice", HW, |a, c, app| {
        let res = crate::orch::system::toggle_audio_device().await;
        let mut ev = json!({ "type": "audio_device" });
        js::assign(&mut ev, res.clone());
        app.emit(ev);
        ok(res)
    });
    reg!("setAudioDevice", HW, |a, c, app| {
        let res = crate::orch::system::set_audio_device(&js::s(&arg(&a, 0))).await;
        let mut ev = json!({ "type": "audio_device" });
        js::assign(&mut ev, res.clone());
        app.emit(ev);
        ok(res)
    });
    // Volume de la voix de Jarvis : appliqué tout de suite, enregistré (config « audio »).
    reg!("jarvisVolume", HUD, |a, c, app| {
        let pct = app.vl().set_jarvis_volume(js::num(&arg(&a, 0)).unwrap_or(0.0));
        let o = app.o();
        o.config.update(&json!({ "audio": { "jarvisVolume": pct } }));
        if arg(&a, 1) == json!(true) {
            app.emit(json!({ "type": "say", "text": format!("Volume de ma voix : {pct} pour cent.") }));
        }
        app.gateway.broadcast(&json!({ "type": "jarvis_volume", "value": pct }), Some(&["hud"]));
        ok(json!({ "ok": true, "jarvisVolume": pct }))
    });
    // Empreinte vocale : phrase d'une voix non reconnue, confirmée depuis le HUD.
    reg!("speakerConfirm", HUD, |a, c, app| ok(app.vl().confirm_speaker(&js::s(&arg(&a, 0)), &json!({ "id": js::s(&arg(&a, 1)) }))));
    reg!("speakerCreate", HUD, |a, c, app| {
        ok(app.vl().confirm_speaker(&js::s(&arg(&a, 0)), &json!({ "create": true, "name": js::s(&arg(&a, 1)), "type": js::s(&arg(&a, 2)) })))
    });
    reg!("speakerDismiss", HUD, |a, c, app| ok(app.vl().dismiss_speaker(&js::s(&arg(&a, 0)))));
    // Bouton APPRENTISSAGE de la zone conversation : même réglage que dans l'administration.
    reg!("speakerLearning", HUD, |a, c, app| {
        let admin = app.admin.borrow().clone().unwrap();
        ok(admin.speaker_options(&json!({ "learning": arg(&a, 0) == json!(true), "origin": "hud" })))
    });
    reg!("speakerUndo", HUD, |a, c, app| {
        let sample = js::s(&arg(&a, 0));
        if !crate::re!(r"^v\d{1,9}$").is_match(&sample) {
            return ok(json!({ "ok": false }));
        }
        app.vl().send(json!({ "t": "speaker_unlearn", "sample": sample }));
        ok(json!({ "ok": true }))
    });
    reg!("clearMemory", HW, |a, c, app| ok(app.o().request_memory_purge()));
    reg!("quit", HW, |a, c, app| {
        shell("quit", json!({}));
        ok(Value::Null)
    });
    // Modules (modules/<id>/) : liste pour le HUD (interface à charger) et l'administration ; activation par l'administration.
    reg!("modulesList", &["hud", "widget", "admin"], |a, c, app| ok(app.registry.list()));
    reg!("modulesSet", &["admin"], |a, c, app| {
        let id = js::s(&arg(&a, 0));
        let on = arg(&a, 1) == json!(true);
        let r = app.registry.set_enabled(&id, on);
        if js::gb(&r, "ok") {
            app.o().record("reglages_modifies", json!({ "sections": ["modules"], "module": id, "actif": on, "origine": "administration" }));
            // Les fenêtres rechargent la liste : l'interface du module apparaît ou disparaît aussitôt.
            app.gateway.broadcast(&json!({ "type": "modules_changed" }), None);
        }
        ok(r)
    });
    // Card « Create module » : conversation avec l'IA pour créer, modifier ou supprimer ses propres modules (data/user-modules/).
    reg!("moduleStudio", HUD, |a, c, app| {
        let op = js::s(&arg(&a, 0));
        if op == "ouvrir" {
            // Bouton de la card Actions : la fenêtre du HUD affiche la card.
            app.gateway.broadcast(&json!({ "type": "creer_module_open" }), None);
            return ok(json!({ "ok": true }));
        }
        let o = app.o();
        let r = app.studio.dispatch(&op, &arg(&a, 1), o.claude_path(), &app.registry).await;
        if js::gb(&r, "changed") {
            o.record("reglages_modifies", json!({ "sections": ["modules"], "operation": op, "origine": "create-module" }));
            app.gateway.broadcast(&json!({ "type": "modules_changed" }), None);
        }
        ok(card_say(&o, r))
    });
    // Administration : fenêtre « admin » seulement, méthodes en liste blanche.
    reg!("admin", &["admin"], |a, c, app| {
        let method = js::s(&arg(&a, 0));
        let admin = app.admin.borrow().clone().unwrap();
        admin.call(&method, a.get(1..).map(|s| s.to_vec()).unwrap_or_default()).await
    });
}

/// Point d'entrée du noyau (sans sous-commande).
pub fn main(modules: Vec<Rc<dyn Module>>) -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[noyau] démarrage impossible : {e}");
            return 1;
        }
    };
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, run(modules))
}

async fn run(modules: Vec<Rc<dyn Module>>) -> i32 {
    let test = js::env("JARVIS_TEST").is_some();
    // Ports propres à Jarvis (v3 et v4 utilisent les mêmes : un seul tourne à la fois).
    if js::env("JARVIS_APPROVAL_PORT").is_none() {
        std::env::set_var("JARVIS_APPROVAL_PORT", "8799");
    }
    let gateway_port = env_num("JARVIS_GATEWAY_PORT", 8797);
    let engine_port = env_num("JARVIS_ENGINE_PORT", 8798);
    let token = js::env("JARVIS_GATEWAY_TOKEN").unwrap_or_else(|| random_hex(24));

    // Données hors du programme (%LOCALAPPDATA%\Jarvis\donnees) : recopie de l'ancien <racine>/data au premier
    // lancement (la coquille l'a d'ordinaire déjà faite), et import d'une archive en attente (transfer.rs).
    let relocated = if test { Ok(crate::paths::Relocated::Nothing) } else { crate::paths::relocate_guarded() };
    let data_dir = crate::paths::data_dir();
    let workspace = crate::paths::workspace();
    let _ = std::fs::create_dir_all(data_dir.join("history"));
    let _ = std::fs::create_dir_all(&workspace);
    crate::logger::init(data_dir.join("logs"));
    match &relocated {
        Ok(crate::paths::Relocated::Copied { files, bytes, kept }) => crate::logger::logger().info(
            "noyau:donnees",
            &format!("{files} fichier(s) ({} Mo) recopiés dans {} ; ancien dossier : {}", bytes / 1_048_576, data_dir.display(), kept.as_ref().map_or("non renommé".into(), |k| k.display().to_string())),
        ),
        Ok(crate::paths::Relocated::Imported { kept }) => crate::logger::logger().info(
            "noyau:donnees",
            &format!("archive importée dans {} ; données remplacées gardées : {}", data_dir.display(), kept.as_ref().map_or("aucune".into(), |k| k.display().to_string())),
        ),
        Err(err) => crate::logger::logger().warn("noyau:donnees", &format!("migration des données reportée ({err}) : dossier {} utilisé", data_dir.display())),
        _ => {}
    }

    // Un seul noyau à la fois, vérifié avant tout ce qui lit ou écrit les données (journal scellé,
    // mémoire, micro). Les tests, qui ont leurs propres données temporaires, n'ont pas le verrou global.
    let global = !test && js::env("JARVIS_TEST_INSTANCE").is_none();
    let lock = crate::instance_lock::InstanceLock::acquire(&data_dir, global, 10000, json!({ "port": gateway_port }), &|m| crate::logger::logger().warn("noyau:instance", &m));
    let _lock = match lock {
        Ok(l) => l,
        Err((holder, _)) => {
            let reason = format!("Jarvis est déjà lancé ({}) : second lancement refusé", crate::instance_lock::describe_holder(&holder));
            crate::logger::logger().warn("noyau:instance", &reason);
            write(&format!("[noyau] {reason}"));
            shell("fatal", json!({ "reason": reason }));
            return 3;
        }
    };

    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        crate::logger::logger().critical("noyau:panic", &msg);
        let _ = writeln!(std::io::stdout(), "[fatal] erreur non gérée : {msg}");
    }));

    let gateway = Gateway::new(gateway_port, token, Rc::new(|m: String| write(&format!("[passerelle] {m}"))), test, crate::paths::root());
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let app = Rc::new(App {
        test,
        data_dir: data_dir.clone(),
        gateway,
        orch: RefCell::new(None),
        supervisor: RefCell::new(None),
        voice: RefCell::new(None),
        code_watch: RefCell::new(None),
        backup: RefCell::new(None),
        updater: RefCell::new(None),
        admin: RefCell::new(None),
        metrics: RefCell::new(None),
        studio: crate::module_studio::Studio::new(),
        registry: Rc::new(Registry::new(crate::paths::root().join("modules"), &data_dir, modules.iter().map(|m| m.id().to_string()).collect())),
        modules,
        started_modules: RefCell::new(vec![]),
        core_bridge: RefCell::new(None),
        metrics_timer: Timer::new(),
        restarting: Cell::new(false),
        stopping: Cell::new(false),
        last_voice_config: RefCell::new(Value::Null),
        done: RefCell::new(Some(done_tx)),
    });

    let emit_app = Rc::downgrade(&app);
    let emit: Rc<dyn Fn(J)> = Rc::new(move |ev: J| {
        if let Some(a) = emit_app.upgrade() {
            a.emit(ev);
        }
    });
    let sbox = crate::secret_box::SecretBox::new();
    let agy_config = (!test && js::env("JARVIS_AGY_CONFIG").as_deref() != Some("0"))
        .then(|| js::env("USERPROFILE").map(|h| PathBuf::from(h).join(".gemini").join("config").join("mcp_config.json")))
        .flatten();
    let orch = Orchestrator::new(OrchOptions {
        workspace: workspace.clone(),
        data_dir: data_dir.clone(),
        models_dir: None,
        emit,
        log: Rc::new(|m: String| write(&format!("[orchestrateur] {m}"))),
        secret_box: Some(Rc::new(sbox)),
        python: js::env("JARVIS_PYTHON"),
        agy_config_path: agy_config,
        notes_dir: None,
        meetings_dir: None,
    });
    *app.orch.borrow_mut() = Some(orch.clone());

    let (o1, o2) = (orch.clone(), orch.clone());
    let sup = VoiceSupervisor::new(
        engine_port,
        data_dir.join("logs"),
        Box::new(move || o1.config.get("voice")),
        Box::new(move || {
            let d = o2.status.d.borrow();
            json!({ "voice": d.voice, "voiceRate": d.voice_rate })
        }),
        Rc::new(|m: String| write(&m)),
    );
    {
        let w = Rc::downgrade(&app);
        *sup.on_status.borrow_mut() = Some(Rc::new(move |st: J| {
            let Some(a) = w.upgrade() else { return };
            let state = js::gs(&st, "state");
            let mut ev = json!({ "type": "voice_backend", "backend": if state == "paused" { "none" } else { "engine" } });
            js::assign(&mut ev, st.clone());
            a.gateway.broadcast(&ev, None);
            if state == "failed" {
                a.o().record("voix_arret", json!({ "raison": st["reason"] }));
            }
        }));
    }
    *app.supervisor.borrow_mut() = Some(sup.clone());

    let ui_app = Rc::downgrade(&app);
    let voice = VoiceLink::new(
        sup.clone(),
        orch.clone(),
        Rc::new(move |ev: J| {
            if let Some(a) = ui_app.upgrade() {
                a.gateway.broadcast(&ev, None);
            }
        }),
        Rc::new(|m: String| write(&m)),
    );
    *app.voice.borrow_mut() = Some(voice.clone());

    // Le diagnostic de démarrage attend le service vocal : « prêt » seulement quand il l'est vraiment.
    {
        let (s, v) = (Rc::downgrade(&sup), Rc::downgrade(&voice));
        *orch.voice_probe.borrow_mut() = Some(Rc::new(move || {
            let (Some(s), Some(v)) = (s.upgrade(), v.upgrade()) else { return "off".into() };
            if js::env("JARVIS_NO_VOICE").is_some() || s.paused.get() || s.gave_up.borrow().is_some() {
                return "off".into();
            }
            if v.is_ready() { "ready".into() } else { "wait".into() }
        }));
    }

    let cw_app = Rc::downgrade(&app);
    let code_watch = CodeWatch::new(
        crate::paths::root(),
        Rc::new(move |st: J| {
            if let Some(a) = cw_app.upgrade() {
                a.gateway.broadcast(&st, None);
            }
        }),
        Rc::new(|m: String| write(&m)),
    );
    *app.code_watch.borrow_mut() = Some(code_watch.clone());

    let sup_w = Rc::downgrade(&sup);
    let metrics = Rc::new(Metrics::new(Box::new(move || Roots {
        shell: js::env("JARVIS_SHELL_PID").and_then(|p| p.parse().ok()),
        core: std::process::id(),
        engine: sup_w.upgrade().and_then(|s| s.pid()),
    })));
    *app.metrics.borrow_mut() = Some(metrics.clone());

    let cfg_app = Rc::downgrade(&app);
    let m2 = metrics.clone();
    let admin = AdminService::new(
        orch.clone(),
        gateway_port,
        sup.clone(),
        Box::new(move || m2.collect()),
        Rc::new(move |changed: &[String], cfg: &J| {
            if let Some(a) = cfg_app.upgrade() {
                a.on_config_change(changed, cfg);
            }
        }),
    );
    *app.admin.borrow_mut() = Some(admin);

    register_methods(&app);
    app.start_modules();
    {
        let w = Rc::downgrade(&app);
        *app.gateway.on_connect.borrow_mut() = Some(Rc::new(move |ws: &Rc<Client>| {
            if let Some(a) = w.upgrade() {
                a.on_connect(ws);
            }
        }));
    }

    // --- Démarrage ---
    if let Err(e) = app.gateway.start().await {
        write(&format!("[noyau] port {gateway_port} indisponible ({e}) : une autre instance tourne déjà"));
        shell("fatal", json!({ "reason": format!("port {gateway_port} occupé") }));
        return 2;
    }
    orch.start();
    app.record_migration();
    {
        // Page d'aide : /aide/donnees.json est calculé à la demande par l'orchestrateur.
        // Les modules installés s'y ajoutent (fiche, card, état), jamais leurs données.
        let w = Rc::downgrade(&orch);
        let reg = app.registry.clone();
        *app.gateway.data_provider.borrow_mut() = Some(Rc::new(move |path: &str| {
            let o = w.upgrade()?;
            (path == "aide/donnees.json").then(|| {
                let mut d = o.help_data();
                d["modules"] = reg.list()["modules"].clone();
                d.to_string()
            })
        }));
        // Images des listes : adresse de l'API d'images et son jeton (lecture seule), donnés aux cards et à la page d'aide.
        crate::orch::collections::schema::set_image_base(&format!("http://127.0.0.1:{}/api/listes/image", app.gateway.port), &app.gateway.image_token());
        // Listes à fiches : le contenu et les formulaires de la page d'aide (jeton, hôte et origine déjà contrôlés
        // par la passerelle). Seules ces opérations sont exposées ; rien d'autre du noyau ne l'est.
        let w = Rc::downgrade(&orch);
        *app.gateway.api.borrow_mut() = Some(Rc::new(move |method: String, path: String, body: J| -> Fut<crate::gateway::ApiReply> {
            let w = w.clone();
            Box::pin(async move {
                use crate::gateway::ApiReply;
                let Some(o) = w.upgrade() else { return ApiReply::json(503, &json!({ "ok": false, "erreur": "noyau arrêté" })) };
                if let Some(rest) = path.strip_prefix("listes/image/") {
                    return match o.collections.image_file(rest) {
                        Some((bytes, ctype)) => ApiReply { status: 200, ctype, body: bytes },
                        None => ApiReply::json(404, &json!({ "ok": false })),
                    };
                }
                let op = path.strip_prefix("listes/").unwrap_or("");
                if method != "POST" || !crate::orch::collections::service::HTTP_OPS.contains(&op) {
                    return ApiReply::json(404, &json!({ "ok": false, "erreur": "opération inconnue" }));
                }
                ApiReply::json(200, &o.collections.dispatch(op, &body, "aide").await)
            })
        }));
    }
    if !test {
        // Sauvegarde automatique de data/ (réglage « backup » : enabled, dir).
        let (oc, orc) = (orch.clone(), orch.clone());
        let backup = crate::backup::Backup::new(
            data_dir.clone(),
            Box::new(move || oc.config.get("backup")),
            Rc::new(move |d| {
                orc.record("sauvegarde", d);
            }),
            Rc::new(|m: String| write(&m)),
        );
        backup.start();
        *app.backup.borrow_mut() = Some(backup);
        // Nouvelles versions publiées (installation seulement ; réglage « update » : mode auto, demander, manuel).
        let (ou, ue) = (orch.clone(), Rc::downgrade(&app));
        let updater = crate::update::Updater::new(
            Box::new(move || ou.config.get("update")),
            Rc::new(move |ev: J| {
                if let Some(a) = ue.upgrade() {
                    a.gateway.broadcast(&ev, None);
                }
            }),
            Rc::new(|m: String| write(&m)),
        );
        updater.start();
        *app.updater.borrow_mut() = Some(updater);
        // Installé : pas de sources à surveiller (les nouvelles versions passent par update.rs).
        if crate::paths::installed().is_none() {
            code_watch.start();
        }
        app.report_relaunch();
    }
    *app.last_voice_config.borrow_mut() = orch.config.get("voice");
    sup.paused.set(!js::truthy(&orch.performance_settings()["voiceListening"]));
    sup.start();
    voice.connect();
    app.set_metrics_log(js::truthy(&orch.effective_performance()["metricsLog"]));
    {
        let w = Rc::downgrade(&app);
        *orch.on_auto_perf_applied.borrow_mut() = Some(Rc::new(move |eff: &J| {
            if let Some(a) = w.upgrade() {
                a.apply_performance(eff);
            }
        }));
    }
    // La coquille (qui a tiré le jeton et l'a déjà injecté dans ses fenêtres) apprend que la
    // passerelle est ouverte et reçoit la configuration du widget.
    shell("ready", json!({ "port": gateway_port, "widget": orch.config.get("widget"), "desk": orch.config.get("desk") }));
    write(&format!("[noyau] prêt · passerelle 127.0.0.1:{gateway_port} · moteur vocal 127.0.0.1:{engine_port}"));

    // Entrée standard : commandes de la coquille ; sa fermeture arrête le noyau.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Option<String>>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    if tx.send(Some(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(None);
    });
    let standalone = js::env("JARVIS_STANDALONE").is_some();
    let in_app = Rc::downgrade(&app);
    js::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let Some(a) = in_app.upgrade() else { break };
            let Some(line) = msg else {
                if !standalone {
                    a.shutdown("coquille fermée");
                }
                break;
            };
            let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
            let o = a.o();
            match js::gs(&m, "cmd").as_str() {
                "quit" => a.shutdown("demande de la coquille"),
                "restart_voice" => a.sup().restart(),
                "widget_ghost" => {
                    let ghost = !js::truthy(&o.config.get("widget")["ghost"]);
                    o.config.update(&json!({ "widget": { "ghost": ghost } }));
                    o.record("reglages_modifies", json!({ "sections": ["widget"], "fantome": ghost, "origine": "raccourci" }));
                    a.on_config_change(&["widget".to_string()], &o.config.all());
                }
                c @ ("desk_toggle" | "desk_set") => {
                    let enabled = if c == "desk_set" { js::truthy(&m["enabled"]) } else { !js::truthy(&o.config.get("desk")["enabled"]) };
                    o.config.update(&json!({ "desk": { "enabled": enabled } }));
                    o.record("reglages_modifies", json!({ "sections": ["desk"], "bureauJarvis": enabled, "origine": "coquille" }));
                    a.on_config_change(&["desk".to_string()], &o.config.all());
                }
                _ => {}
            }
        }
    });
    let sig_app = Rc::downgrade(&app);
    js::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            if let Some(a) = sig_app.upgrade() {
                a.shutdown("SIGINT");
            }
        }
    });

    let _ = done_rx.await;
    0
}
