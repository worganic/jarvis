// Canal Telegram (orchestrator/telegram.js) : parler à Jarvis depuis le téléphone et recevoir ses
// notifications. Bot privé (jeton chiffré par DPAPI), appairage par un code à 6 chiffres affiché dans
// l'administration ; seule la conversation appairée est écoutée. Un message reçu devient une demande
// (source « telegram ») ; un code de validation n'y est jamais accepté.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use rand::Rng;
use serde_json::{json, Value};

use super::{OrchRef, Up};
use crate::js::{self, J};
use crate::secret_box::SecretBox;

const ACTIVE_MS: i64 = 10 * 60 * 1000;
const PAIRING_MS: i64 = 15 * 60 * 1000;
const MAX_TEXT: usize = 3900;
const API_BASE: &str = "https://api.telegram.org";

pub fn rows_text(rows: &J) -> String {
    js::arr(rows)
        .iter()
        .filter(|r| js::truthy(&r["title"]) || js::truthy(&r["section"]))
        .take(25)
        .map(|r| {
            if js::truthy(&r["section"]) {
                format!("\n{}", js::gs(r, "section"))
            } else {
                let sub = js::gs(r, "sub");
                let meta = js::gs(r, "meta");
                format!("• {}{}{}", js::gs(r, "title"), if sub.is_empty() { String::new() } else { format!(" — {sub}") }, if meta.is_empty() { String::new() } else { format!(" ({meta})") })
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

pub struct Telegram {
    me: Weak<Telegram>,
    o: OrchRef,
    file: PathBuf,
    secret_box: Option<Rc<SecretBox>>,
    log: Rc<dyn Fn(String)>,
    pub config: RefCell<J>,
    offset: Cell<i64>,
    pub running: Cell<bool>,
    generation: Cell<u64>,
    pairing: RefCell<Option<(String, i64)>>,
    active_until: Cell<i64>,
    refused: RefCell<HashSet<i64>>,
    queue: tokio::sync::Mutex<()>,
    pub last_error: RefCell<String>,
    listening: Cell<bool>,
}

impl Telegram {
    pub fn new(o: OrchRef, file: PathBuf, secret_box: Option<Rc<SecretBox>>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let t = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            o,
            file,
            secret_box,
            log,
            config: RefCell::new(json!({})),
            offset: Cell::new(0),
            running: Cell::new(false),
            generation: Cell::new(0),
            pairing: RefCell::new(None),
            active_until: Cell::new(0),
            refused: RefCell::new(HashSet::new()),
            queue: tokio::sync::Mutex::new(()),
            last_error: RefCell::new(String::new()),
            listening: Cell::new(false),
        });
        *t.config.borrow_mut() = t.load();
        t
    }

    fn load(&self) -> J {
        let raw = js::read_json(&self.file).unwrap_or(json!({}));
        let mut cfg = js::spread(&json!({ "token": "", "bot": "", "chatId": null, "chatName": "", "enabled": true, "notifications": true }), &raw);
        if js::truthy(&raw["tokenEnc"]) {
            if let Some(b) = &self.secret_box {
                cfg["token"] = json!(b.decrypt(&js::gs(&raw, "tokenEnc")).unwrap_or_default());
            }
        }
        cfg.as_object_mut().unwrap().remove("tokenEnc");
        cfg
    }

    fn save(&self) {
        let mut stored = self.config.borrow().clone();
        if let (Some(b), true) = (&self.secret_box, js::truthy(&stored["token"])) {
            if let Ok(enc) = b.encrypt(&js::gs(&stored, "token")) {
                stored["tokenEnc"] = json!(enc);
                stored.as_object_mut().unwrap().remove("token");
            }
        }
        let _ = js::write_json(&self.file, &stored);
    }

    fn c(&self, k: &str) -> J {
        self.config.borrow()[k].clone()
    }

    pub fn state(&self) -> J {
        let code = self.pairing.borrow().as_ref().filter(|(_, until)| *until > js::now_ms()).map(|(c, _)| c.clone());
        json!({
            "configured": js::truthy(&self.c("token")), "bot": self.c("bot"), "paired": js::truthy(&self.c("chatId")),
            "chatName": self.c("chatName"), "enabled": self.c("enabled") != json!(false), "notifications": self.c("notifications") != json!(false),
            "running": self.running.get(), "encrypted": self.secret_box.is_some(), "pairingCode": code, "lastError": *self.last_error.borrow()
        })
    }

    pub fn new_pairing_code(&self) -> String {
        let code = rand::thread_rng().gen_range(100000..1000000).to_string();
        *self.pairing.borrow_mut() = Some((code.clone(), js::now_ms() + PAIRING_MS));
        code
    }

    async fn api(&self, method: &str, body: J, token: &str, timeout_s: u64) -> Result<J, String> {
        let res = crate::net::http()
            .post(format!("{API_BASE}/bot{token}/{method}"))
            .json(&body)
            .timeout(std::time::Duration::from_secs(timeout_s))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = res.status().as_u16();
        let data: Option<J> = res.json().await.ok();
        match data {
            Some(d) if js::gb(&d, "ok") => Ok(d["result"].clone()),
            Some(d) if js::truthy(&d["description"]) => Err(js::gs(&d, "description")),
            _ => Err(format!("HTTP {status}")),
        }
    }

    /// Nouveau jeton : vérifié auprès de Telegram (getMe) avant d'être enregistré.
    pub async fn configure(&self, token: &str) -> J {
        let clean = token.trim().to_string();
        if !crate::re!(r"^\d{5,}:[\w-]{30,}$").is_match(&clean) {
            return json!({ "ok": false, "error": "jeton invalide (format 123456789:ABC…, donné par @BotFather)" });
        }
        let me = match self.api("getMe", json!({}), &clean, 15).await {
            Ok(m) => m,
            Err(e) => return json!({ "ok": false, "error": format!("Telegram refuse ce jeton : {e}") }),
        };
        let changed = clean != js::s(&self.c("token"));
        {
            let mut cfg = self.config.borrow_mut();
            cfg["token"] = json!(clean);
            cfg["bot"] = json!(if js::truthy(&me["username"]) { js::gs(&me, "username") } else { js::gs(&me, "first_name") });
            if changed {
                cfg["chatId"] = Value::Null;
                cfg["chatName"] = json!("");
            }
        }
        self.save();
        if !js::truthy(&self.c("chatId")) {
            self.new_pairing_code();
        }
        self.restart();
        js::spread(&json!({ "ok": true }), &self.state())
    }

    pub fn set(&self, options: &J) -> J {
        if let Some(b) = options["enabled"].as_bool() {
            self.config.borrow_mut()["enabled"] = json!(b);
        }
        if let Some(b) = options["notifications"].as_bool() {
            self.config.borrow_mut()["notifications"] = json!(b);
        }
        self.save();
        self.restart();
        js::spread(&json!({ "ok": true }), &self.state())
    }

    pub fn unpair(&self) -> J {
        self.config.borrow_mut()["chatId"] = Value::Null;
        self.config.borrow_mut()["chatName"] = json!("");
        self.save();
        self.new_pairing_code();
        js::spread(&json!({ "ok": true }), &self.state())
    }

    pub fn forget(&self) -> J {
        self.stop();
        *self.config.borrow_mut() = json!({ "token": "", "bot": "", "chatId": null, "chatName": "", "enabled": true, "notifications": true });
        *self.pairing.borrow_mut() = None;
        let _ = std::fs::remove_file(&self.file);
        js::spread(&json!({ "ok": true }), &self.state())
    }

    /// Envoi immédiat, résultat attendu (message de test de l'administration).
    pub async fn send_now(&self, text: &str) -> bool {
        let chat = self.c("chatId");
        let token = js::s(&self.c("token"));
        if !js::truthy(&chat) || token.is_empty() {
            return false;
        }
        match self.api("sendMessage", json!({ "chat_id": chat, "text": text, "disable_web_page_preview": true }), &token, 20).await {
            Ok(_) => true,
            Err(e) => {
                *self.last_error.borrow_mut() = e;
                false
            }
        }
    }

    /// Envois en file : l'ordre est gardé, une erreur n'arrête pas les suivants.
    pub fn send(&self, text: &str, chat: Option<J>) {
        let body = text.trim().to_string();
        let chat = chat.unwrap_or_else(|| self.c("chatId"));
        let token = js::s(&self.c("token"));
        if body.is_empty() || !js::truthy(&chat) || token.is_empty() {
            return;
        }
        let chars: Vec<char> = body.chars().collect();
        let chunks: Vec<String> = chars.chunks(MAX_TEXT).map(|c| c.iter().collect()).collect();
        let me = self.me.clone();
        js::spawn(async move {
            let Some(s) = me.upgrade() else { return };
            let _guard = s.queue.lock().await;
            for chunk in chunks {
                if let Err(e) = s.api("sendMessage", json!({ "chat_id": chat, "text": chunk, "disable_web_page_preview": true }), &token, 20).await {
                    *s.last_error.borrow_mut() = e.clone();
                    (s.log)(format!("Telegram : envoi impossible ({e})"));
                    return;
                }
            }
        });
    }

    pub fn start(&self) {
        if !self.listening.get() {
            self.listening.set(true);
            let o = self.o.up();
            let me = self.me.clone();
            o.notify_listeners.borrow_mut().push(Rc::new(move |n: &J| {
                if let Some(s) = me.upgrade() {
                    s.forward_notification(n);
                }
            }));
            let me = self.me.clone();
            o.emit_listeners.borrow_mut().push(Rc::new(move |e: &J| {
                if let Some(s) = me.upgrade() {
                    s.relay(e);
                }
            }));
        }
        if self.running.get() || !js::truthy(&self.c("token")) || self.c("enabled") == json!(false) {
            return;
        }
        if !js::truthy(&self.c("chatId")) && self.pairing.borrow().is_none() {
            self.new_pairing_code();
        }
        self.running.set(true);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let me = self.me.clone();
        js::spawn(async move {
            if let Some(s) = me.upgrade() {
                s.poll_loop(generation).await;
            }
        });
    }

    pub fn stop(&self) {
        self.running.set(false);
        self.generation.set(self.generation.get() + 1);
    }

    pub fn restart(&self) {
        self.stop();
        let me = self.me.clone();
        js::set_timeout(50, move || {
            if let Some(s) = me.upgrade() {
                if !s.o.up().stopped.get() {
                    s.start();
                }
            }
        });
    }

    async fn poll_loop(self: Rc<Self>, generation: u64) {
        let mut delay = 2000u64;
        while self.running.get() && self.generation.get() == generation {
            let token = js::s(&self.c("token"));
            let res = self.api("getUpdates", json!({ "offset": self.offset.get(), "timeout": 25, "allowed_updates": ["message"] }), &token, 35).await;
            if self.generation.get() != generation || !self.running.get() {
                break;
            }
            match res {
                Ok(updates) => {
                    delay = 2000;
                    self.last_error.borrow_mut().clear();
                    for u in js::arr(&updates) {
                        self.offset.set(self.offset.get().max(u["update_id"].as_i64().unwrap_or(0) + 1));
                        self.handle(&u["message"]).await;
                    }
                }
                Err(e) => {
                    *self.last_error.borrow_mut() = e.clone();
                    (self.log)(format!("Telegram : {e} (nouvel essai dans {} s)", delay / 1000));
                    js::sleep_ms(delay).await;
                    delay = (delay * 2).min(60000);
                }
            }
        }
    }

    async fn handle(&self, message: &J) {
        if !js::truthy(message) || !js::truthy(&message["chat"]) {
            return;
        }
        let chat_id = message["chat"]["id"].clone();
        let text = (if js::truthy(&message["text"]) { js::gs(message, "text") } else { js::gs(message, "caption") }).trim().to_string();
        let names: Vec<String> = [js::gs(&message["from"], "first_name"), js::gs(&message["from"], "last_name")].into_iter().filter(|s| !s.is_empty()).collect();
        let who = if !names.is_empty() { names.join(" ") } else if js::truthy(&message["chat"]["title"]) { js::gs(&message["chat"], "title") } else { js::s(&chat_id) };
        let o = self.o.up();
        if !js::truthy(&self.c("chatId")) {
            let code = self.pairing.borrow().as_ref().filter(|(_, u)| *u > js::now_ms()).map(|(c, _)| c.clone());
            let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            if code.as_deref() == Some(digits.as_str()) {
                self.config.borrow_mut()["chatId"] = chat_id.clone();
                self.config.borrow_mut()["chatName"] = json!(who);
                self.save();
                *self.pairing.borrow_mut() = None;
                o.record("telegram_appaire", json!({ "conversation": who }));
                o.emit(json!({ "type": "say", "text": format!("Telegram est relié, {}.", who.split(' ').next().unwrap_or("")), "ack": true }));
                self.send("✅ Jarvis est relié à cette conversation. Écris-moi ce que tu me dirais à voix haute. Les validations par code se font toujours sur le PC.", Some(chat_id));
            } else {
                self.send("Bonjour. Pour relier ce bot à Jarvis, envoie le code à 6 chiffres affiché dans l’administration (section Telegram).", Some(chat_id));
            }
            return;
        }
        if chat_id != self.c("chatId") {
            let id = chat_id.as_i64().unwrap_or(0);
            if self.refused.borrow_mut().insert(id) {
                o.record("telegram_refuse", json!({ "conversation": who }));
                self.send("Ce Jarvis est privé.", Some(chat_id));
            }
            return;
        }
        if text.is_empty() {
            self.send("Je ne lis que le texte pour l’instant.", None);
            return;
        }
        if crate::re!(r"(?i)^/start\b").is_match(&text) {
            self.send("Je t’écoute. Exemples : « quels sont mes rappels ? », « rappelle-moi dans 20 minutes d’appeler Paul », « fais-moi le briefing », « stop ».", None);
            return;
        }
        if crate::re!(r"(?i)^/stop\b|^(?:stop|arr[eê]te tout)$").is_match(&text) {
            self.active_until.set(js::now_ms() + ACTIVE_MS);
            o.stop_all("telegram");
            self.send("■ Tout est arrêté.", None);
            return;
        }
        self.active_until.set(js::now_ms() + ACTIVE_MS);
        let t = crate::re!(r"^/\w+\s*").replace(&text, "").into_owned();
        o.submit(t, json!({ "source": "telegram" })).await;
    }

    fn active(&self) -> bool {
        js::truthy(&self.c("chatId")) && js::now_ms() < self.active_until.get()
    }

    fn relay(&self, event: &J) {
        if !self.active() {
            return;
        }
        let typ = js::gs(event, "type");
        if typ == "say" && js::truthy(&event["text"]) && !js::truthy(&event["stream"]) && !js::truthy(&event["notification"]) {
            self.send(&js::gs(event, "text"), None);
        } else if typ == "widget" && event["rows"].as_array().map(|r| !r.is_empty()).unwrap_or(false) && !js::truthy(&event["notification"]) {
            let list = rows_text(&event["rows"]);
            if !list.is_empty() {
                let title = js::gs(event, "title");
                self.send(&format!("{}{list}", if title.is_empty() { String::new() } else { format!("{title}\n") }), None);
            }
        }
    }

    fn forward_notification(&self, n: &J) {
        let text = js::gs(n, "text");
        if !js::truthy(&self.c("chatId")) || self.c("notifications") == json!(false) || text.is_empty() {
            return;
        }
        let list = rows_text(&n["rows"]);
        self.send(&format!("🔔 {text}{}", if !list.is_empty() && list != text { format!("\n\n{list}") } else { String::new() }), None);
    }
}
