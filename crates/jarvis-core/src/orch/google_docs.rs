// Documents Google Docs créés par Jarvis (orchestrator/google-docs.js), par l'API Google Drive.
// Connexion OAuth 2 d'une « application de bureau » (ID client + code secret collés dans
// l'administration) : Jarvis ouvre la page de consentement, reçoit la réponse sur http://127.0.0.1:<port>
// (PKCE) et garde le jeton de renouvellement chiffré par Windows (DPAPI). Portée « drive.file » : Jarvis
// ne voit que les fichiers qu'il a créés. Le HTML est converti par Google en Google Doc, rangé dans le
// dossier « Réunions Jarvis ».

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use base64::Engine;
use rand::RngCore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

use crate::js::{self, J};
use crate::secret_box::SecretBox;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DRIVE_URL: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name,webViewLink";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const FOLDER_NAME: &str = "Réunions Jarvis";
const CONSENT_TIMEOUT_MS: u64 = 5 * 60 * 1000;

fn b64url(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn random(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut v);
    v
}

/// ID client et code secret collés tels quels, ou fichier JSON téléchargé depuis Google Cloud Console.
pub fn parse_client(client_id: &str, client_secret: &str) -> (String, String) {
    let raw = client_id.trim();
    if raw.starts_with('{') {
        return match serde_json::from_str::<J>(raw) {
            Ok(j) => {
                let c = if j["installed"].is_object() { &j["installed"] } else if j["web"].is_object() { &j["web"] } else { &j };
                (js::gs(c, "client_id").trim().to_string(), js::gs(c, "client_secret").trim().to_string())
            }
            Err(_) => (String::new(), String::new()),
        };
    }
    (raw.to_string(), client_secret.trim().to_string())
}

fn page(title: &str, text: &str) -> String {
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Jarvis</title>\n<body style=\"font:16px system-ui;background:#04121c;color:#bfefff;display:grid;place-items:center;height:100vh;margin:0\">\n<div style=\"text-align:center\"><h2 style=\"color:#38e1ff\">{title}</h2><p>{text}</p></div>"
    )
}

#[derive(Clone, Default)]
pub struct DocsConfig {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub email: String,
    pub folder_id: String,
    pub connected_at: i64,
}

/// Erreur d'API : message, et « setup » quand il faut (re)connecter Jarvis.
pub struct DocsError {
    pub message: String,
    pub setup: bool,
}

impl From<String> for DocsError {
    fn from(message: String) -> Self {
        Self { message, setup: false }
    }
}

pub struct GoogleDocs {
    file: PathBuf,
    sbox: Option<Rc<SecretBox>>,
    open_url: Rc<dyn Fn(String)>,
    log: Rc<dyn Fn(String)>,
    token: RefCell<Option<(String, i64)>>,
    waiters: RefCell<Option<Vec<oneshot::Sender<J>>>>,
    pub config: RefCell<DocsConfig>,
}

impl GoogleDocs {
    pub fn new(file: PathBuf, sbox: Option<Rc<SecretBox>>, open_url: Rc<dyn Fn(String)>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let sbox = sbox.filter(|b| b.available());
        let me = Rc::new(Self { file, sbox, open_url, log, token: RefCell::new(None), waiters: RefCell::new(None), config: RefCell::new(DocsConfig::default()) });
        let cfg = me.load();
        *me.config.borrow_mut() = cfg;
        me
    }

    // --- Réglages -------------------------------------------------------------------------------

    fn load(&self) -> DocsConfig {
        let raw = js::read_json(&self.file).unwrap_or(json!({}));
        let open = |enc: &str, clear: &str| -> String {
            match (&self.sbox, js::truthy(&raw[enc])) {
                (Some(b), true) => b.decrypt(&js::gs(&raw, enc)).unwrap_or_default(),
                _ => js::gs(&raw, clear),
            }
        };
        DocsConfig {
            client_id: js::gs(&raw, "clientId"),
            client_secret: open("clientSecretEnc", "clientSecret"),
            refresh_token: open("refreshTokenEnc", "refreshToken"),
            email: js::gs(&raw, "email"),
            folder_id: js::gs(&raw, "folderId"),
            connected_at: js::gn_or(&raw, "connectedAt", 0.0) as i64,
        }
    }

    fn save(&self) {
        let c = self.config.borrow().clone();
        let mut stored = json!({ "clientId": c.client_id, "email": c.email, "folderId": c.folder_id, "connectedAt": c.connected_at });
        for (key, value) in [("clientSecret", &c.client_secret), ("refreshToken", &c.refresh_token)] {
            if value.is_empty() {
                continue;
            }
            match self.sbox.as_ref().and_then(|b| b.encrypt(value).ok()) {
                Some(enc) => stored[format!("{key}Enc")] = json!(enc),
                None => stored[key] = json!(value),
            }
        }
        if let Err(e) = js::write_json(&self.file, &stored) {
            (self.log)(format!("Google Docs : enregistrement impossible ({e})"));
        }
    }

    pub fn state(&self) -> J {
        let c = self.config.borrow();
        json!({
            "hasClient": !c.client_id.is_empty() && !c.client_secret.is_empty(), "clientId": c.client_id,
            "connected": !c.client_id.is_empty() && !c.refresh_token.is_empty(), "email": c.email, "connectedAt": c.connected_at,
            "encrypted": self.sbox.is_some(), "waiting": self.waiters.borrow().is_some()
        })
    }

    pub fn configured(&self) -> bool {
        let c = self.config.borrow();
        !c.client_id.is_empty() && !c.refresh_token.is_empty()
    }

    pub fn set_client(&self, client_id: &str, client_secret: &str) -> J {
        let (id, secret) = parse_client(client_id, client_secret);
        let current = self.config.borrow().clone();
        let secret = if secret.is_empty() && id == current.client_id { current.client_secret.clone() } else { secret };
        if !crate::re!(r"^[\w.-]+\.apps\.googleusercontent\.com$").is_match(&id) {
            return json!({ "ok": false, "error": "ID client invalide : il se termine par « .apps.googleusercontent.com »." });
        }
        if secret.is_empty() {
            return json!({ "ok": false, "error": "Code secret du client manquant." });
        }
        let changed = id != current.client_id;
        *self.token.borrow_mut() = None;
        {
            let mut c = self.config.borrow_mut();
            c.client_id = id;
            c.client_secret = secret;
            if changed {
                c.refresh_token.clear();
                c.email.clear();
                c.folder_id.clear();
                c.connected_at = 0;
            }
        }
        self.save();
        json!({ "ok": true })
    }

    pub fn forget(&self) -> J {
        *self.token.borrow_mut() = None;
        *self.config.borrow_mut() = DocsConfig::default();
        let _ = std::fs::remove_file(&self.file);
        json!({ "ok": true })
    }

    // --- Connexion (consentement dans le navigateur) -----------------------------------------------

    /// Résout quand Google a répondu (ou au bout de 5 minutes).
    pub async fn connect(self: &Rc<Self>) -> J {
        let c = self.config.borrow().clone();
        if c.client_id.is_empty() || c.client_secret.is_empty() {
            return json!({ "ok": false, "error": "Renseigne d’abord l’ID client et le code secret." });
        }
        let (tx, rx) = oneshot::channel();
        let already = {
            let mut w = self.waiters.borrow_mut();
            match w.as_mut() {
                Some(list) => {
                    list.push(tx);
                    true
                }
                None => {
                    *w = Some(vec![tx]);
                    false
                }
            }
        };
        if !already {
            let me = self.clone();
            js::spawn(async move {
                let r = me.run_consent(c).await;
                let list = me.waiters.borrow_mut().take().unwrap_or_default();
                for w in list {
                    let _ = w.send(r.clone());
                }
            });
        }
        rx.await.unwrap_or(json!({ "ok": false, "error": "connexion interrompue" }))
    }

    async fn run_consent(self: &Rc<Self>, c: DocsConfig) -> J {
        let verifier = b64url(&random(32));
        let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
        let state = b64url(&random(16));
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let redirect = format!("http://127.0.0.1:{port}");
        let params = [
            ("client_id", c.client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", &format!("{SCOPE} email")),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
        ]
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");
        (self.open_url)(format!("{AUTH_URL}?{params}"));
        let serve = async {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { continue };
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let url = reqwest::Url::parse(&format!("http://127.0.0.1{path}")).ok();
                let reply = |title: &str, text: &str| {
                    let body = page(title, text);
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                };
                let Some(url) = url.filter(|u| u.path() == "/") else {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    continue;
                };
                let q: std::collections::HashMap<String, String> = url.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
                if q.get("state").map(|s| s.as_str()) != Some(state.as_str()) {
                    let _ = sock.write_all(reply("Réponse ignorée", "Cette réponse ne vient pas de la demande de Jarvis.").as_bytes()).await;
                    continue;
                }
                let Some(code) = q.get("code").cloned() else {
                    let _ = sock.write_all(reply("Connexion refusée", "Google n’a pas donné l’accès. Tu peux fermer cet onglet.").as_bytes()).await;
                    return json!({ "ok": false, "error": format!("accès refusé ({})", q.get("error").cloned().unwrap_or_else(|| "sans code".into())) });
                };
                let result = async {
                    let tok = self.token_request(&[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect), ("code_verifier", &verifier)]).await?;
                    let refresh = js::gs(&tok, "refresh_token");
                    if refresh.is_empty() {
                        return Err("Google n’a pas fourni de jeton de renouvellement".to_string());
                    }
                    let expires = js::now_ms() + (js::gn_or(&tok, "expires_in", 3600.0) as i64) * 1000;
                    *self.token.borrow_mut() = Some((js::gs(&tok, "access_token"), expires));
                    let email = self.user_email().await.unwrap_or_default();
                    {
                        let mut c = self.config.borrow_mut();
                        c.refresh_token = refresh;
                        c.email = email.clone();
                        c.connected_at = js::now_ms();
                    }
                    self.save();
                    Ok(email)
                }
                .await;
                return match result {
                    Ok(email) => {
                        let _ = sock.write_all(reply("Jarvis est relié à Google Docs", "Tu peux fermer cet onglet et revenir à Jarvis.").as_bytes()).await;
                        json!({ "ok": true, "email": email })
                    }
                    Err(e) => {
                        let _ = sock.write_all(reply("Connexion impossible", &e).as_bytes()).await;
                        json!({ "ok": false, "error": e })
                    }
                };
            }
        };
        match tokio::time::timeout(std::time::Duration::from_millis(CONSENT_TIMEOUT_MS), serve).await {
            Ok(r) => r,
            Err(_) => json!({ "ok": false, "error": "délai dépassé (5 min) : connexion non terminée" }),
        }
    }

    async fn token_request(&self, params: &[(&str, &str)]) -> Result<J, String> {
        let c = self.config.borrow().clone();
        let mut form: Vec<(&str, &str)> = vec![("client_id", &c.client_id), ("client_secret", &c.client_secret)];
        form.extend_from_slice(params);
        let res = crate::net::http().post(TOKEN_URL).form(&form).send().await.map_err(|e| e.to_string())?;
        let ok = res.status().is_success();
        let status = res.status().as_u16();
        let j: J = res.json().await.unwrap_or(json!({}));
        if !ok {
            let m = if js::truthy(&j["error_description"]) { js::gs(&j, "error_description") } else if js::truthy(&j["error"]) { js::gs(&j, "error") } else { format!("Google : {status}") };
            return Err(m);
        }
        Ok(j)
    }

    async fn access_token(&self) -> Result<String, DocsError> {
        if let Some((t, exp)) = self.token.borrow().clone() {
            if exp - js::now_ms() > 60000 {
                return Ok(t);
            }
        }
        if !self.configured() {
            return Err(DocsError { message: "Google Docs n’est pas relié à Jarvis".into(), setup: true });
        }
        let refresh = self.config.borrow().refresh_token.clone();
        match self.token_request(&[("grant_type", "refresh_token"), ("refresh_token", &refresh)]).await {
            Ok(tok) => {
                let t = js::gs(&tok, "access_token");
                *self.token.borrow_mut() = Some((t.clone(), js::now_ms() + (js::gn_or(&tok, "expires_in", 3600.0) as i64) * 1000));
                Ok(t)
            }
            Err(e) if crate::re!(r"(?i)invalid_grant").is_match(&e) => {
                {
                    let mut c = self.config.borrow_mut();
                    c.refresh_token.clear();
                    c.connected_at = 0;
                }
                self.save();
                Err(DocsError { message: "l’accès à Google Docs a expiré : reconnecte Jarvis dans l’administration".into(), setup: true })
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn api(&self, req: reqwest::RequestBuilder) -> Result<J, DocsError> {
        let token = self.access_token().await?;
        let res = req.bearer_auth(token).send().await.map_err(|e| e.to_string())?;
        let ok = res.status().is_success();
        let status = res.status().as_u16();
        let j: J = res.json().await.unwrap_or(json!({}));
        if !ok {
            let m = js::gs(&j["error"], "message");
            return Err(if m.is_empty() { format!("Google Drive : {status}") } else { m }.into());
        }
        Ok(j)
    }

    async fn user_email(&self) -> Result<String, DocsError> {
        let j = self.api(crate::net::http().get("https://www.googleapis.com/oauth2/v3/userinfo")).await?;
        Ok(js::gs(&j, "email"))
    }

    // --- Documents ------------------------------------------------------------------------------

    async fn folder(&self) -> Result<String, DocsError> {
        let fid = self.config.borrow().folder_id.clone();
        if !fid.is_empty() {
            if let Ok(f) = self.api(crate::net::http().get(format!("{DRIVE_URL}/{}?fields=id,trashed", urlencoding::encode(&fid)))).await {
                if js::truthy(&f["id"]) && !js::gb(&f, "trashed") {
                    return Ok(js::gs(&f, "id"));
                }
            }
        }
        let q = format!("name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false", FOLDER_NAME.replace('\'', "\\'"));
        let found = self.api(crate::net::http().get(format!("{DRIVE_URL}?q={}&fields=files(id)&spaces=drive", urlencoding::encode(&q)))).await?;
        let mut id = js::gs(&found["files"][0], "id");
        if id.is_empty() {
            let made = self
                .api(crate::net::http().post(DRIVE_URL).header("Content-Type", "application/json").body(json!({ "name": FOLDER_NAME, "mimeType": "application/vnd.google-apps.folder" }).to_string()))
                .await?;
            id = js::gs(&made, "id");
        }
        self.config.borrow_mut().folder_id = id.clone();
        self.save();
        Ok(id)
    }

    /// HTML converti par Google en Google Doc. → { ok, id, url, name } ou { ok: false, error, setup }
    pub async fn create_document(&self, title: &str, html: &str) -> J {
        let r: Result<J, DocsError> = async {
            let parent = self.folder().await?;
            let boundary = format!("jarvis{}", hex::encode(random(8)));
            let meta = json!({ "name": js::slice_chars(if title.is_empty() { "Réunion" } else { title }, 200), "mimeType": "application/vnd.google-apps.document", "parents": [parent] });
            let body = [
                format!("--{boundary}"),
                "Content-Type: application/json; charset=UTF-8".into(),
                String::new(),
                meta.to_string(),
                format!("--{boundary}"),
                "Content-Type: text/html; charset=UTF-8".into(),
                String::new(),
                html.to_string(),
                format!("--{boundary}--"),
                String::new(),
            ]
            .join("\r\n");
            let doc = self.api(crate::net::http().post(UPLOAD_URL).header("Content-Type", format!("multipart/related; boundary={boundary}")).body(body)).await?;
            Ok(doc)
        }
        .await;
        match r {
            Ok(doc) => {
                let name = js::gs(&doc, "name");
                (self.log)(format!("Google Docs : document créé ({name})"));
                let id = js::gs(&doc, "id");
                let url = if js::truthy(&doc["webViewLink"]) { js::gs(&doc, "webViewLink") } else { format!("https://docs.google.com/document/d/{id}/edit") };
                json!({ "ok": true, "id": id, "url": url, "name": name })
            }
            Err(e) => json!({ "ok": false, "error": e.message, "setup": e.setup }),
        }
    }
}

#[allow(unused)]
fn _v(_: Value) {}
