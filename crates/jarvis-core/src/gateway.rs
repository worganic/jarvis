// Passerelle interface ↔ noyau (core/gateway.js de la v3).
//
// WebSocket lié à 127.0.0.1, jeton aléatoire tiré à chaque lancement et injecté par la coquille
// dans ses propres fenêtres uniquement. Trois contrôles cumulés :
//   1. jeton exact (comparaison à temps constant) ;
//   2. origine : seules les pages de la coquille Tauri ;
//   3. rôle : la fenêtre « admin » est la seule à pouvoir appeler le service d'administration.
// Le même port sert en HTTP (GET/HEAD) les fichiers de ui/ et tools/ (aperçu des personnages).
// Protocole inchangé : requête { id, m, a } → réponse { id, r } ou { id, e } ; événement { ev }.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::Message;

use crate::js::{self, J};
use crate::orch::Fut;

const ALLOWED_ORIGINS: &[&str] = &["http://tauri.localhost", "https://tauri.localhost", "tauri://localhost"];
const ROLES: &[&str] = &["hud", "widget", "admin"];

fn static_type(ext: &str) -> &'static str {
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "glb" => "model/gltf-binary",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// Comparaison à temps constant.
fn same_token(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    x.len() == y.len() && x.iter().zip(y).fold(0u8, |acc, (p, q)| acc | (p ^ q)) == 0
}

fn query_param(uri: &str, key: &str) -> Option<String> {
    let q = uri.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then(|| urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_default())
    })
}

/// Fenêtre connectée.
pub struct Client {
    pub id: u64,
    pub role: String,
    tx: mpsc::UnboundedSender<Message>,
}

impl Client {
    fn send_text(&self, text: String) {
        let _ = self.tx.send(Message::Text(text.into()));
    }
}

/// Réponse d'une route d'API (page d'aide : listes à fiches).
pub struct ApiReply {
    pub status: u16,
    pub ctype: &'static str,
    pub body: Vec<u8>,
}

impl ApiReply {
    pub fn json(status: u16, v: &J) -> Self {
        Self { status, ctype: "application/json; charset=utf-8", body: v.to_string().into_bytes() }
    }
}

/// Route d'API : (méthode, chemin sous « api/ », corps JSON) → réponse. L'authentification est faite avant.
pub type ApiHandler = Rc<dyn Fn(String, String, J) -> Fut<ApiReply>>;

/// Taille maximale du corps d'une requête d'API.
const API_BODY_MAX: usize = 1_048_576;

/// Jeton dérivé du jeton de la passerelle : « aide » (page d'aide, lecture et écriture des listes) ou
/// « img » (images des listes, lecture seule, passé en paramètre d'adresse puisqu'une balise <img> n'a pas d'en-tête).
pub fn derived_token(token: &str, purpose: &str) -> String {
    sha1_smol::Sha1::from(format!("{token}:{purpose}:jarvis")).hexdigest()
}

/// Méthode offerte aux fenêtres : arguments, fenêtre appelante → résultat ou message d'erreur.
pub type Method = Rc<dyn Fn(Vec<J>, Rc<Client>) -> Fut<Result<J, String>>>;

pub struct Gateway {
    me: RefCell<Weak<Gateway>>,
    pub port: u16,
    token: String,
    log: Rc<dyn Fn(String)>,
    allow_no_origin: bool,
    root: PathBuf,
    methods: RefCell<HashMap<String, (Method, Vec<String>)>>,
    clients: RefCell<Vec<Rc<Client>>>,
    seq: Cell<u64>,
    pub on_connect: RefCell<Option<Rc<dyn Fn(&Rc<Client>)>>>,
    /// Données dynamiques servies en JSON (chemin sous « aide/ » → texte JSON), ex. aide/donnees.json.
    pub data_provider: RefCell<Option<Rc<dyn Fn(&str) -> Option<String>>>>,
    /// Routes /api/… (données des listes : page d'aide et images), protégées par jeton.
    pub api: RefCell<Option<ApiHandler>>,
    server: RefCell<Option<tokio::task::JoinHandle<()>>>,
}

impl Gateway {
    pub fn new(port: u16, token: String, log: Rc<dyn Fn(String)>, allow_no_origin: bool, root: PathBuf) -> Rc<Self> {
        let g = Rc::new(Self {
            me: RefCell::new(Weak::new()),
            port,
            token,
            log,
            allow_no_origin,
            root,
            methods: RefCell::new(HashMap::new()),
            clients: RefCell::new(vec![]),
            seq: Cell::new(0),
            on_connect: RefCell::new(None),
            data_provider: RefCell::new(None),
            api: RefCell::new(None),
            server: RefCell::new(None),
        });
        *g.me.borrow_mut() = Rc::downgrade(&g);
        g
    }

    pub fn aide_token(&self) -> String {
        derived_token(&self.token, "aide")
    }

    pub fn image_token(&self) -> String {
        derived_token(&self.token, "img")
    }

    /// `host` et `origin` : en-têtes de la requête. Une page d'un autre site (ou un rebond DNS) ne passe pas.
    pub fn host_ok(&self, host: Option<&str>, origin: Option<&str>) -> bool {
        let p = self.port;
        let host_ok = matches!(host, Some(h) if h == format!("127.0.0.1:{p}") || h == format!("localhost:{p}"));
        let origin_ok = match origin {
            None => true,
            Some(o) => o == format!("http://127.0.0.1:{p}") || o == format!("http://localhost:{p}"),
        };
        host_ok && origin_ok
    }

    /// Contrôle d'accès d'une route d'API : Err(code HTTP) si refusée.
    #[allow(clippy::too_many_arguments)]
    pub fn api_check(&self, method: &str, rest: &str, host: Option<&str>, origin: Option<&str>, header_token: Option<&str>, query_token: Option<&str>, content_type: Option<&str>) -> Result<(), u16> {
        if !self.host_ok(host, origin) {
            return Err(403);
        }
        if rest.starts_with("listes/image/") {
            if method != "GET" && method != "HEAD" {
                return Err(405);
            }
            return match query_token {
                Some(t) if same_token(t, &self.image_token()) => Ok(()),
                _ => Err(403),
            };
        }
        if !rest.starts_with("listes/") {
            return Err(404);
        }
        if method != "POST" {
            return Err(405);
        }
        match header_token {
            Some(t) if same_token(t, &self.aide_token()) => {}
            _ => return Err(401),
        }
        if !content_type.map(|c| c.to_ascii_lowercase().starts_with("application/json")).unwrap_or(false) {
            return Err(415);
        }
        Ok(())
    }

    pub fn has_method(&self, name: &str) -> bool {
        self.methods.borrow().contains_key(name)
    }

    /// roles : rôles autorisés à appeler la méthode (par défaut hud et widget).
    pub fn register<F>(&self, name: &str, roles: &[&str], f: F)
    where
        F: Fn(Vec<J>, Rc<Client>) -> Fut<Result<J, String>> + 'static,
    {
        let roles = if roles.is_empty() { vec!["hud".to_string(), "widget".to_string()] } else { roles.iter().map(|r| r.to_string()).collect() };
        self.methods.borrow_mut().insert(name.to_string(), (Rc::new(f), roles));
    }

    /// Variante synchrone de `register`.
    pub fn register_sync<F>(&self, name: &str, roles: &[&str], f: F)
    where
        F: Fn(Vec<J>, Rc<Client>) -> Result<J, String> + 'static,
    {
        self.register(name, roles, move |a, c| {
            let r = f(a, c);
            Box::pin(async move { r })
        });
    }

    /// Ouvre le port ; Err si déjà pris (une autre instance tourne).
    pub async fn start(&self) -> Result<(), String> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", self.port)).await.map_err(|e| e.to_string())?;
        let me = self.me.borrow().clone();
        let task = js::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { continue };
                let Some(g) = me.upgrade() else { break };
                js::spawn(async move { g.on_stream(stream).await });
            }
        });
        *self.server.borrow_mut() = Some(task);
        Ok(())
    }

    /// Demande d'ouverture WebSocket, ou simple requête HTTP ? Lu sans consommer les octets.
    async fn is_websocket(stream: &tokio::net::TcpStream) -> bool {
        let mut buf = vec![0u8; 8192];
        for _ in 0..50 {
            let n = match stream.peek(&mut buf).await {
                Ok(0) | Err(_) => return false,
                Ok(n) => n,
            };
            let head = String::from_utf8_lossy(&buf[..n]).to_lowercase();
            if head.contains("\r\n\r\n") || n == buf.len() {
                return head.lines().any(|l| l.starts_with("upgrade:") && l.contains("websocket"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }

    async fn on_stream(self: Rc<Self>, stream: tokio::net::TcpStream) {
        if Self::is_websocket(&stream).await {
            self.accept_ws(stream).await;
        } else {
            self.serve_http(stream).await;
        }
    }

    async fn accept_ws(self: Rc<Self>, stream: tokio::net::TcpStream) {
        let role = Rc::new(RefCell::new(String::new()));
        let role_slot = role.clone();
        let (token, allow, log) = (self.token.clone(), self.allow_no_origin, self.log.clone());
        let cb = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
            let uri = req.uri().to_string();
            let origin = req.headers().get("origin").and_then(|v| v.to_str().ok()).map(String::from);
            let origin_ok = match &origin {
                Some(o) => ALLOWED_ORIGINS.contains(&o.as_str()),
                None => allow,
            };
            let r = query_param(&uri, "role").unwrap_or_default();
            let ok = origin_ok && same_token(&query_param(&uri, "token").unwrap_or_default(), &token) && ROLES.contains(&r.as_str());
            if !ok {
                log(format!("connexion refusée (origine {})", origin.unwrap_or_else(|| "aucune".into())));
                let mut e = ErrorResponse::new(None);
                *e.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
                return Err(e);
            }
            *role_slot.borrow_mut() = r;
            Ok(resp)
        };
        let cfg = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(4 * 1024 * 1024));
        let Ok(ws) = tokio_tungstenite::accept_hdr_async_with_config(stream, cb, Some(cfg)).await else { return };
        let (mut sink, mut source) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let id = self.seq.get() + 1;
        self.seq.set(id);
        let client = Rc::new(Client { id, role: role.borrow().clone(), tx });
        self.clients.borrow_mut().push(client.clone());
        let writer = js::spawn(async move {
            while let Some(m) = rx.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
        });
        let cb = self.on_connect.borrow().clone();
        if let Some(f) = cb {
            f(&client);
        }
        while let Some(Ok(msg)) = source.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            let me = self.clone();
            let c = client.clone();
            js::spawn(async move { me.on_message(c, &text).await });
        }
        writer.abort();
        self.clients.borrow_mut().retain(|c| c.id != id);
    }

    async fn on_message(&self, client: Rc<Client>, raw: &str) {
        let Ok(msg) = serde_json::from_str::<Value>(raw) else { return };
        let id = msg.get("id").cloned();
        let m = js::gs(&msg, "m");
        let reply = |payload: J| {
            if let Some(id) = &id {
                let mut out = json!({ "id": id });
                js::assign(&mut out, payload);
                client.send_text(out.to_string());
            }
        };
        let entry = self.methods.borrow().get(&m).cloned();
        let Some((f, roles)) = entry.filter(|(_, roles)| roles.contains(&client.role)) else {
            reply(json!({ "e": format!("méthode refusée : {m}") }));
            return;
        };
        let _ = roles;
        let args = msg["a"].as_array().cloned().unwrap_or_default();
        match f(args, client.clone()).await {
            Ok(r) => reply(json!({ "r": r })),
            Err(e) => reply(json!({ "e": e })),
        }
    }

    async fn serve_http(self: Rc<Self>, stream: tokio::net::TcpStream) {
        use http_body_util::Full;
        use hyper::body::Bytes;
        let root = self.root.clone();
        let me = self.clone();
        let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
            let root = root.clone();
            let me = me.clone();
            async move {
                let res = |code: u16| hyper::Response::builder().status(code).body(Full::new(Bytes::new())).unwrap();
                let method = req.method().clone();
                if let Some(rest) = req.uri().path().strip_prefix("/api/") {
                    let rest = rest.to_string();
                    let reply = me.serve_api(req, rest).await;
                    return Ok::<_, std::convert::Infallible>(
                        hyper::Response::builder()
                            .status(reply.status)
                            .header("Content-Type", reply.ctype)
                            .header("Cache-Control", "no-store")
                            .header("X-Content-Type-Options", "nosniff")
                            .body(Full::new(Bytes::from(reply.body)))
                            .unwrap(),
                    );
                }
                if method != hyper::Method::GET && method != hyper::Method::HEAD {
                    return Ok::<_, std::convert::Infallible>(res(405));
                }
                let Ok(pathname) = urlencoding::decode(req.uri().path()) else { return Ok(res(400)) };
                let mut rel = pathname.trim_start_matches('/').replace('\\', "/");
                if rel.is_empty() || rel == "preview" {
                    rel = "tools/character-preview.html".into();
                }
                // Données vivantes de la page d'aide (jamais de fichier : calculées par le noyau).
                if rel.starts_with("aide/") && !rel.contains("..") {
                    let host = req.headers().get("host").and_then(|v| v.to_str().ok());
                    if !me.host_ok(host, None) {
                        return Ok(res(403));
                    }
                    let provider = me.data_provider.borrow().clone();
                    return match provider.and_then(|p| p(&rel)) {
                        Some(json) => Ok(hyper::Response::builder()
                            .status(200)
                            .header("Content-Type", "application/json; charset=utf-8")
                            .header("Cache-Control", "no-store")
                            .body(Full::new(if method == hyper::Method::HEAD { Bytes::new() } else { Bytes::from(json) }))
                            .unwrap()),
                        None => Ok(res(404)),
                    };
                }
                let top = rel.split('/').next().unwrap_or("");
                // Seuls ui/, tools/ et l'interface des modules (modules/<id>/ui/) sont servis (pas data/, crates/, etc.)
                let module_ui = top == "modules" && rel.split('/').nth(2) == Some("ui");
                if !(["ui", "tools"].contains(&top) || module_ui) || rel.split('/').any(|p| p == ".." || p.contains(':')) {
                    return Ok(res(403));
                }
                // Modules créés par l'utilisateur : leur interface vit dans data/user-modules/<id>/ui/.
                let parts: Vec<&str> = rel.split('/').collect();
                let file = if module_ui && parts.get(1).map(|id| id.starts_with(crate::modules::USER_PREFIX)).unwrap_or(false) {
                    crate::modules::user_dir(&crate::paths::data_dir()).join(parts[1..].join("/"))
                } else {
                    root.join(&rel)
                };
                let Ok(buf) = tokio::fs::read(&file).await else { return Ok(res(404)) };
                let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                let body = if method == hyper::Method::HEAD { Bytes::new() } else { Bytes::from(buf) };
                Ok(hyper::Response::builder()
                    .status(200)
                    .header("Content-Type", static_type(&ext))
                    .header("Cache-Control", "no-store")
                    .body(Full::new(body))
                    .unwrap())
            }
        });
        let io = hyper_util::rt::TokioIo::new(stream);
        let _ = hyper::server::conn::http1::Builder::new().serve_connection(io, svc).await;
    }

    /// Route /api/… : contrôle d'accès, lecture du corps JSON (1 Mo au plus), appel du gestionnaire.
    async fn serve_api(self: &Rc<Self>, req: hyper::Request<hyper::body::Incoming>, rest: String) -> ApiReply {
        use http_body_util::BodyExt;
        let header = |n: &str| req.headers().get(n).and_then(|v| v.to_str().ok()).map(String::from);
        let (host, origin, token, ctype) = (header("host"), header("origin"), header("x-jarvis-aide"), header("content-type"));
        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        let query_token = query_param(&query, "t");
        let method = req.method().as_str().to_string();
        if let Err(code) = self.api_check(&method, &rest, host.as_deref(), origin.as_deref(), token.as_deref(), query_token.as_deref(), ctype.as_deref()) {
            if code == 403 || code == 401 {
                (self.log)(format!("API refusée ({code}) : {method} /api/{} (hôte {})", rest, host.unwrap_or_default()));
            }
            return ApiReply::json(code, &json!({ "ok": false, "erreur": "accès refusé" }));
        }
        let body: J = if method == "POST" {
            let Ok(collected) = http_body_util::Limited::new(req.into_body(), API_BODY_MAX).collect().await else {
                return ApiReply::json(413, &json!({ "ok": false, "erreur": "requête trop grande" }));
            };
            match serde_json::from_slice::<J>(&collected.to_bytes()) {
                Ok(v) if v.is_object() => v,
                _ => return ApiReply::json(400, &json!({ "ok": false, "erreur": "JSON invalide" })),
            }
        } else {
            Value::Null
        };
        let handler = self.api.borrow().clone();
        match handler {
            Some(h) => h(method, rest, body).await,
            None => ApiReply::json(503, &json!({ "ok": false, "erreur": "indisponible" })),
        }
    }

    /// Événement vers toutes les fenêtres (ou celles des rôles donnés).
    pub fn broadcast(&self, event: &J, roles: Option<&[&str]>) {
        let data = json!({ "ev": event }).to_string();
        for c in self.clients.borrow().iter() {
            if roles.map(|r| r.contains(&c.role.as_str())).unwrap_or(true) {
                c.send_text(data.clone());
            }
        }
    }

    pub fn send(&self, client: &Client, event: &J) {
        client.send_text(json!({ "ev": event }).to_string());
    }

    pub fn count(&self, role: Option<&str>) -> usize {
        self.clients.borrow().iter().filter(|c| role.map(|r| c.role == r).unwrap_or(true)).count()
    }

    pub fn stop(&self) {
        if let Some(t) = self.server.borrow_mut().take() {
            t.abort();
        }
        for c in self.clients.borrow_mut().drain(..) {
            let _ = c.tx.send(Message::Close(None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gw() -> Rc<Gateway> {
        Gateway::new(8797, "jeton-de-test".into(), Rc::new(|_| {}), false, PathBuf::from("."))
    }

    #[test]
    fn les_jetons_derives_sont_distincts() {
        let g = gw();
        assert_ne!(g.aide_token(), g.image_token());
        assert_ne!(g.aide_token(), "jeton-de-test");
        assert_eq!(g.aide_token().len(), 40);
    }

    #[test]
    fn controle_d_acces_de_l_api() {
        let g = gw();
        let (aide, img) = (g.aide_token(), g.image_token());
        let ok = |m: &str, p: &str, host: Option<&str>, origin: Option<&str>, h: Option<&str>, q: Option<&str>, ct: Option<&str>| g.api_check(m, p, host, origin, h, q, ct);
        let host = Some("127.0.0.1:8797");
        let json = Some("application/json");
        // Cas normal
        assert_eq!(ok("POST", "listes/lister", host, None, Some(&aide), None, json), Ok(()));
        assert_eq!(ok("POST", "listes/lister", Some("localhost:8797"), Some("http://localhost:8797"), Some(&aide), None, json), Ok(()));
        // Pas de jeton, mauvais jeton, jeton des images
        assert_eq!(ok("POST", "listes/lister", host, None, None, None, json), Err(401));
        assert_eq!(ok("POST", "listes/lister", host, None, Some("faux"), None, json), Err(401));
        assert_eq!(ok("POST", "listes/lister", host, None, Some(&img), None, json), Err(401), "le jeton des images n'ouvre pas l'API");
        // Rebond DNS et autre site
        assert_eq!(ok("POST", "listes/lister", Some("evil.example:8797"), None, Some(&aide), None, json), Err(403));
        assert_eq!(ok("POST", "listes/lister", None, None, Some(&aide), None, json), Err(403));
        assert_eq!(ok("POST", "listes/lister", host, Some("https://evil.example"), Some(&aide), None, json), Err(403));
        assert_eq!(ok("POST", "listes/lister", Some("127.0.0.1:9999"), None, Some(&aide), None, json), Err(403));
        // Méthode et type de contenu
        assert_eq!(ok("GET", "listes/lister", host, None, Some(&aide), None, json), Err(405));
        assert_eq!(ok("POST", "listes/lister", host, None, Some(&aide), None, Some("text/plain")), Err(415));
        assert_eq!(ok("POST", "listes/lister", host, None, Some(&aide), None, None), Err(415));
        // Hors des listes : rien d'autre n'est exposé
        assert_eq!(ok("POST", "memoire/lire", host, None, Some(&aide), None, json), Err(404));
        // Images : GET avec le jeton des images seulement
        assert_eq!(ok("GET", "listes/image/a/b.jpg", host, None, None, Some(&img), None), Ok(()));
        assert_eq!(ok("GET", "listes/image/a/b.jpg", host, None, None, Some(&aide), None), Err(403));
        assert_eq!(ok("GET", "listes/image/a/b.jpg", host, None, None, None, None), Err(403));
        assert_eq!(ok("POST", "listes/image/a/b.jpg", host, None, Some(&aide), Some(&img), json), Err(405));
        assert_eq!(ok("GET", "listes/image/a/b.jpg", Some("evil.example:8797"), None, None, Some(&img), None), Err(403));
    }
}
