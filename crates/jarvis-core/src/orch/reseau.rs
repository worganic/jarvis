// Jarvis Réseau : deux Jarvis amis échangent des messages, des listes, des fichiers et leur présence (cœur / avatar)
// par le service web-reseau/ (PHP, hébergement du site Nomade). Calqué sur nomade/mod.rs : uniquement des appels sortants,
// relève toutes les 45 s (toutes les 4 s pendant deux minutes quand la card Amis est ouverte ou qu'un message part).
//
// Compte : un mail vérifié par code ; Jarvis génère une paire de clés X25519 et ne publie que la clé publique. Tout
// contenu envoyé est chiffré pour le destinataire (boîte NaCl : crypto_box, X25519 + XSalsa20-Poly1305) : le serveur ne
// le lit pas. Clé secrète et clé d'appareil sont chiffrées par Windows (DPAPI) dans data/reseau.json, et suivent
// l'archive .jarvis (rechiffrées par son mot de passe, transfer.rs).
//
// Rien n'est partagé par défaut : présence, chaque liste (lecture ou écriture) et chaque dossier se partagent ami par ami.
// Ce qui arrive est rangé dans data/reseau/ (messages, listes reçues en lecture seule, fichiers reçus jamais exécutés).
// Une modification de liste par un ami n'est appliquée que si cette liste lui est partagée en écriture, et passe par
// collections/service.rs comme toute autre modification.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use base64::Engine;
use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use serde_json::{json, Value as J};
use sha2::{Digest, Sha256};

use super::{OrchRef, Up};
use crate::js::{self, Timer};
use crate::secret_box::SecretBox;

const SLOW_MS: u64 = 45_000;
const FAST_MS: u64 = 4_000;
const FAST_FOR_MS: i64 = 120_000;
const PRESENCE_EVERY_MS: i64 = 60_000;
/// Messages gardés par ami.
const KEEP_MESSAGES: usize = 500;
/// Fichiers d'un dossier envoyé : 1,4 Mo par envoi (2 Mo une fois chiffré et encodé), 25 Mo au plus par dossier.
const CHUNK: usize = 1_400_000;
const FOLDER_MAX: u64 = 25 * 1024 * 1024;
/// Opérations qu'un ami peut faire sur une liste partagée en écriture (collections/service.rs › dispatch).
const FRIEND_LIST_OPS: &[&str] = &["ajouter_element", "modifier_element", "supprimer_element", "changer_statut"];

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn hash(v: &J) -> String {
    hex::encode(Sha256::digest(v.to_string().as_bytes()))
}

/// Empreinte lisible d'une clé publique (à comparer de vive voix) : 4 groupes de 4 caractères.
pub fn fingerprint(public_b64: &str) -> String {
    let h = hex::encode(Sha256::digest(public_b64.as_bytes())).to_uppercase();
    h.as_bytes().chunks(4).take(4).map(|c| String::from_utf8_lossy(c).to_string()).collect::<Vec<_>>().join("-")
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_alphanumeric() || " .-_()".contains(c) { c } else { '_' }).collect();
    let n = n.trim().trim_matches('.').to_string();
    if n.is_empty() { "sans-nom".into() } else { n.chars().take(120).collect() }
}

/// Chiffre `content` pour `their` (clé publique base64) avec notre clé secrète. → (nonce, contenu) en base64.
pub fn seal(mine: &SecretKey, their: &str, content: &J) -> Result<(String, String), String> {
    let pk = public_key(their)?;
    let sbox = SalsaBox::new(&pk, mine);
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let ct = sbox.encrypt(&nonce, content.to_string().as_bytes()).map_err(|_| "chiffrement impossible".to_string())?;
    Ok((b64().encode(nonce), b64().encode(ct)))
}

/// Déchiffre un envoi de `their` (clé publique base64 de l'expéditeur).
pub fn open(mine: &SecretKey, their: &str, nonce: &str, content: &str) -> Result<J, String> {
    let pk = public_key(their)?;
    let sbox = SalsaBox::new(&pk, mine);
    let n = b64().decode(nonce).map_err(|e| e.to_string())?;
    if n.len() != 24 {
        return Err("nonce invalide".into());
    }
    let ct = b64().decode(content).map_err(|e| e.to_string())?;
    let clear = sbox.decrypt(crypto_box::Nonce::from_slice(&n), ct.as_slice()).map_err(|_| "envoi illisible (clé changée ?)".to_string())?;
    serde_json::from_slice(&clear).map_err(|e| e.to_string())
}

fn public_key(b: &str) -> Result<PublicKey, String> {
    let raw = b64().decode(b.trim()).map_err(|e| e.to_string())?;
    let arr: [u8; 32] = raw.try_into().map_err(|_| "clé publique invalide".to_string())?;
    Ok(PublicKey::from(arr))
}

pub struct Reseau {
    me: Weak<Reseau>,
    o: OrchRef,
    file: PathBuf,
    dir: PathBuf,
    secret_box: Option<Rc<SecretBox>>,
    log: Rc<dyn Fn(String)>,
    /// data/reseau.json : serveur, compte, clés (DPAPI), dernier envoi relevé, partages notés, empreintes envoyées.
    state: RefCell<J>,
    /// Dernière liste d'amis du serveur.
    friends: RefCell<Vec<J>>,
    friends_at: Cell<i64>,
    timer: Timer,
    fast_until: Cell<i64>,
    busy: Cell<bool>,
    last_presence: Cell<i64>,
    last_error: RefCell<String>,
    pending_email: RefCell<Option<(String, String, String)>>,
    listening: Cell<bool>,
}

impl Reseau {
    pub fn new(o: OrchRef, file: PathBuf, dir: PathBuf, secret_box: Option<Rc<SecretBox>>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let state = js::read_json(&file).unwrap_or(json!({}));
        Rc::new_cyclic(|me| Self {
            me: me.clone(),
            o,
            file,
            dir,
            secret_box,
            log,
            state: RefCell::new(state),
            friends: RefCell::new(vec![]),
            friends_at: Cell::new(-1),
            timer: Timer::new(),
            fast_until: Cell::new(0),
            busy: Cell::new(false),
            last_presence: Cell::new(0),
            last_error: RefCell::new(String::new()),
            pending_email: RefCell::new(None),
            listening: Cell::new(false),
        })
    }

    fn s(&self, k: &str) -> String {
        js::gs(&self.state.borrow(), k)
    }

    fn save(&self) {
        let _ = js::write_json(&self.file, &self.state.borrow());
    }

    fn reveal(&self, k: &str) -> Option<String> {
        let enc = self.s(k);
        if enc.is_empty() {
            return None;
        }
        self.secret_box.as_ref().and_then(|b| b.decrypt(&enc).ok())
    }

    fn protect(&self, clear: &str) -> Result<String, String> {
        self.secret_box.as_ref().ok_or("chiffrement Windows indisponible")?.encrypt(clear)
    }

    fn secret_key(&self) -> Option<SecretKey> {
        let raw = b64().decode(self.reveal("cleSecrete")?).ok()?;
        let arr: [u8; 32] = raw.try_into().ok()?;
        Some(SecretKey::from(arr))
    }

    pub fn connected(&self) -> bool {
        !self.s("serveur").is_empty() && !self.s("cleAppareil").is_empty()
    }

    // --- Appels au service ------------------------------------------------------------------------------------------

    fn url(&self, server: &str, route: &str) -> String {
        format!("{}/api/?r={route}", server.trim_end_matches('/'))
    }

    async fn call(&self, method: &str, route: &str, body: Option<J>) -> Result<J, String> {
        self.call_to(&self.s("serveur"), method, route, body, true).await
    }

    async fn call_to(&self, server: &str, method: &str, route: &str, body: Option<J>, auth: bool) -> Result<J, String> {
        if server.is_empty() {
            return Err("adresse du service Jarvis Réseau manquante".into());
        }
        let http = crate::net::http();
        let mut req = if method == "GET" { http.get(self.url(server, route)) } else { http.post(self.url(server, route)).json(&body.unwrap_or(json!({}))) };
        req = req.header("X-Jarvis-Version", env!("CARGO_PKG_VERSION")).timeout(std::time::Duration::from_secs(30));
        if auth {
            let key = self.reveal("cleAppareil").ok_or("compte non relié")?;
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let res = req.send().await.map_err(|e| format!("service injoignable : {e}"))?;
        let status = res.status();
        let v: J = res.json().await.map_err(|_| format!("réponse illisible du service ({status})"))?;
        if !js::gb(&v, "ok") {
            return Err(Some(js::gs(&v, "error")).filter(|e| !e.is_empty()).unwrap_or_else(|| format!("refus du service ({status})")));
        }
        Ok(v)
    }

    // --- Inscription ------------------------------------------------------------------------------------------------

    /// Étape 1 : clés générées sur ce PC, code envoyé au mail.
    pub async fn register(&self, server: &str, email: &str, name: &str) -> J {
        let server = server.trim().trim_end_matches('/').to_string();
        if !server.starts_with("https://") {
            return json!({ "ok": false, "error": "L'adresse du service doit commencer par https://" });
        }
        let secret = SecretKey::generate(&mut OsRng);
        let public = b64().encode(secret.public_key().as_bytes());
        let secret_b64 = b64().encode(secret.to_bytes());
        match self.call_to(&server, "POST", "inscription", Some(json!({ "email": email.trim(), "nom": name.trim(), "cle": public })), false).await {
            Ok(r) => {
                *self.pending_email.borrow_mut() = Some((server, email.trim().to_lowercase(), secret_b64));
                json!({ "ok": true, "minutes": r["minutes"] })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    /// Étape 2 : code du mail → clé d'appareil ; compte relié.
    pub async fn verify(&self, code: &str) -> J {
        let Some((server, email, secret_b64)) = self.pending_email.borrow().clone() else {
            return json!({ "ok": false, "error": "Demande d'abord un code." });
        };
        let label = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
        match self.call_to(&server, "POST", "verification", Some(json!({ "email": email, "code": code.trim(), "appareil": label })), false).await {
            Ok(r) => {
                let (dev, sec) = match (self.protect(&js::gs(&r, "cleAppareil")), self.protect(&secret_b64)) {
                    (Ok(d), Ok(s)) => (d, s),
                    (Err(e), _) | (_, Err(e)) => return json!({ "ok": false, "error": e }),
                };
                {
                    let mut st = self.state.borrow_mut();
                    st["serveur"] = json!(server);
                    st["email"] = json!(email);
                    st["compte"] = r["compte"].clone();
                    st["cleAppareil"] = json!(dev);
                    st["cleSecrete"] = json!(sec);
                    st["depuis"] = json!(0);
                }
                self.save();
                *self.pending_email.borrow_mut() = None;
                self.o.up().record("reseau_compte", json!({ "email": email, "action": "relié" }));
                self.restart();
                self.fast(FAST_FOR_MS);
                json!({ "ok": true, "etat": self.view() })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    /// Oublie le compte sur ce PC (`delete` : le supprime aussi du service, amis et envois compris).
    pub async fn forget(&self, delete: bool) -> J {
        if delete && self.connected() {
            if let Err(e) = self.call("POST", "compte-supprimer", None).await {
                return json!({ "ok": false, "error": e });
            }
        }
        self.timer.clear();
        let email = self.s("email");
        *self.state.borrow_mut() = json!({ "serveur": self.s("serveur") });
        self.save();
        self.friends.borrow_mut().clear();
        self.o.up().record("reseau_compte", json!({ "email": email, "action": if delete { "supprimé" } else { "oublié sur ce PC" } }));
        json!({ "ok": true, "etat": self.view() })
    }

    // --- Boucle -----------------------------------------------------------------------------------------------------

    pub fn start(&self) {
        if !self.listening.get() {
            self.listening.set(true);
            let me = self.me.clone();
            self.o.up().emit_listeners.borrow_mut().push(Rc::new(move |e: &J| {
                if js::gs(e, "type") == "collections_changed" {
                    if let Some(s) = me.upgrade() {
                        s.fast(20_000);
                    }
                }
            }));
        }
        self.restart();
    }

    fn restart(&self) {
        self.timer.clear();
        if !self.connected() {
            return;
        }
        let me = self.me.clone();
        self.timer.every(FAST_MS, move || {
            let Some(s) = me.upgrade() else { return };
            // Relève lente, sauf pendant la fenêtre « rapide » (card ouverte, message envoyé).
            let now = js::now_ms();
            let due = now < s.fast_until.get() || now - js::gn(&s.state.borrow(), "releveLe").unwrap_or(0.0) as i64 >= SLOW_MS as i64;
            if due && !s.busy.get() {
                js::spawn(async move {
                    s.cycle().await;
                });
            }
        });
    }

    /// Relève plus fréquente pendant `ms` (card Amis ouverte, conversation en cours).
    pub fn fast(&self, ms: i64) {
        self.fast_until.set(self.fast_until.get().max(js::now_ms() + ms));
    }

    async fn cycle(&self) {
        self.busy.set(true);
        let res = self.cycle_inner().await;
        self.busy.set(false);
        self.state.borrow_mut()["releveLe"] = json!(js::now_ms());
        let changed = match res {
            Ok(changed) => {
                let had = !self.last_error.borrow().is_empty();
                self.last_error.borrow_mut().clear();
                changed || had
            }
            Err(e) => {
                let first = *self.last_error.borrow() != e;
                if first {
                    (self.log)(format!("[réseau] {e}"));
                }
                *self.last_error.borrow_mut() = e;
                first
            }
        };
        if changed {
            self.o.up().emit(json!({ "type": "reseau", "etat": self.view() }));
        }
    }

    /// Une relève : amis (si changés), envois reçus, présence, listes partagées modifiées. Vrai si quelque chose change.
    async fn cycle_inner(&self) -> Result<bool, String> {
        let since = js::gn(&self.state.borrow(), "depuis").unwrap_or(0.0) as i64;
        let r = self.call("GET", &format!("recevoir&depuis={since}"), None).await?;
        let mut changed = false;
        let at = js::gn(&r, "amisMajLe").unwrap_or(0.0) as i64;
        if at != self.friends_at.get() {
            self.refresh_friends().await?;
            self.friends_at.set(at);
            changed = true;
        }
        let items = js::arr(&r["envois"]);
        let mut last = since;
        for it in &items {
            if let Err(e) = self.receive(it).await {
                (self.log)(format!("[réseau] envoi {} ignoré : {e}", js::gs(it, "id")));
            }
            last = last.max(js::gn(it, "id").unwrap_or(0.0) as i64);
        }
        if !items.is_empty() {
            self.state.borrow_mut()["depuis"] = json!(last);
            self.save();
            let _ = self.call("POST", "accuse", Some(json!({ "jusqua": last }))).await;
            changed = true;
        }
        if js::now_ms() - self.last_presence.get() >= PRESENCE_EVERY_MS {
            self.last_presence.set(js::now_ms());
            let _ = self.call("POST", "presence", Some(self.presence())).await;
        }
        if self.push_shared_lists().await > 0 {
            changed = true;
        }
        Ok(changed)
    }

    async fn refresh_friends(&self) -> Result<(), String> {
        let r = self.call("GET", "amis", None).await?;
        let list = js::arr(&r["amis"]);
        // Nouvelle demande reçue : signalée une fois.
        let known: Vec<i64> = self.friends.borrow().iter().map(|f| js::gn(f, "id").unwrap_or(0.0) as i64).collect();
        for f in &list {
            let id = js::gn(f, "id").unwrap_or(0.0) as i64;
            if js::gs(f, "statut") == "pending" && !js::gb(f, "envoyee") && !known.contains(&id) && self.friends_at.get() >= 0 {
                let name = js::gs(&f["ami"], "nom");
                self.o.up().emit(json!({ "type": "reseau_demande", "nom": name }));
                self.o.up().record("reseau_demande_recue", json!({ "de": name, "email": f["ami"]["email"] }));
            }
        }
        *self.friends.borrow_mut() = list;
        Ok(())
    }

    fn friend(&self, account_id: i64) -> Option<J> {
        self.friends.borrow().iter().find(|f| js::gn(&f["ami"], "id").unwrap_or(-1.0) as i64 == account_id && js::gs(f, "statut") == "accepted").cloned()
    }

    fn friend_dir(&self, account_id: i64) -> PathBuf {
        self.dir.join("amis").join(account_id.to_string())
    }

    /// Présence envoyée au service (vue des seuls amis à qui elle est partagée) : jamais d'image ni de fenêtre.
    fn presence(&self) -> J {
        let o = self.o.up();
        let p = o.presence.state();
        let c = o.companion.view();
        let meeting = js::arr(&c["effects"]).iter().any(|e| e.as_str() == Some("meeting"));
        let etat = if js::gb(&self.state.borrow(), "nePasDeranger") || meeting {
            "ne_pas_deranger"
        } else if js::gb(&p, "enabled") && p["present"] == json!(false) {
            "absent"
        } else {
            "en_ligne"
        };
        let avatar = if js::gb(&c, "enabled") { format!("{}:{}", js::gs(&c, "stage"), js::gs(&c, "branch")) } else { String::new() };
        json!({ "etat": etat, "avatar": avatar })
    }

    // --- Envoi ------------------------------------------------------------------------------------------------------

    async fn send_item(&self, to: i64, kind: &str, r#ref: &str, content: &J) -> Result<i64, String> {
        let f = self.friend(to).ok_or("vous n'êtes pas (encore) amis")?;
        let sk = self.secret_key().ok_or("clé secrète absente : relie de nouveau ton compte")?;
        let (nonce, ct) = seal(&sk, &js::gs(&f["ami"], "cle"), content)?;
        let r = self.call("POST", "envoyer", Some(json!({ "a": to, "type": kind, "ref": r#ref, "nonce": nonce, "contenu": ct }))).await?;
        Ok(js::gn(&r, "id").unwrap_or(0.0) as i64)
    }

    pub async fn send_message(&self, to: i64, text: &str) -> J {
        let text: String = text.trim().chars().take(4000).collect();
        if text.is_empty() {
            return json!({ "ok": false, "error": "message vide" });
        }
        let now = js::iso_now();
        match self.send_item(to, "message", "", &json!({ "texte": text, "le": now })).await {
            Ok(id) => {
                self.store_message(to, json!({ "id": id, "moi": true, "texte": text, "le": now }));
                self.fast(FAST_FOR_MS);
                json!({ "ok": true, "messages": self.messages(to) })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    fn messages_file(&self, account_id: i64) -> PathBuf {
        self.friend_dir(account_id).join("messages.json")
    }

    fn store_message(&self, account_id: i64, m: J) {
        let f = self.messages_file(account_id);
        let mut list = js::read_json(&f).map(|v| js::arr(&v)).unwrap_or_default();
        list.push(m);
        if list.len() > KEEP_MESSAGES {
            list.drain(..list.len() - KEEP_MESSAGES);
        }
        if let Some(p) = f.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = js::write_json(&f, &json!(list));
    }

    pub fn messages(&self, account_id: i64) -> J {
        json!(js::read_json(&self.messages_file(account_id)).map(|v| js::arr(&v)).unwrap_or_default())
    }

    /// Conversation lue : accusé de lecture envoyé une fois.
    pub async fn mark_read(&self, account_id: i64) -> J {
        let list = js::arr(&self.messages(account_id));
        let unread: Vec<i64> = list.iter().filter(|m| !js::gb(m, "moi") && !js::gb(m, "lu")).map(|m| js::gn(m, "id").unwrap_or(0.0) as i64).collect();
        if unread.is_empty() {
            return json!({ "ok": true });
        }
        let updated: Vec<J> = list.into_iter().map(|mut m| { if !js::gb(&m, "moi") { m["lu"] = json!(true); } m }).collect();
        let _ = js::write_json(&self.messages_file(account_id), &json!(updated));
        let _ = self.send_item(account_id, "lu", "", &json!({ "ids": unread })).await;
        json!({ "ok": true })
    }

    // --- Réception --------------------------------------------------------------------------------------------------

    async fn receive(&self, it: &J) -> Result<(), String> {
        let from = js::gn(it, "de").unwrap_or(0.0) as i64;
        let f = self.friend(from).ok_or("expéditeur qui n'est pas un ami")?;
        let sk = self.secret_key().ok_or("clé secrète absente")?;
        let c = open(&sk, &js::gs(&f["ami"], "cle"), &js::gs(it, "nonce"), &js::gs(it, "contenu"))?;
        let name = js::gs(&f["ami"], "nom");
        let dir = self.friend_dir(from);
        let _ = std::fs::create_dir_all(&dir);
        match js::gs(it, "type").as_str() {
            "message" => {
                let text = js::gs(&c, "texte");
                self.store_message(from, json!({ "id": it["id"], "moi": false, "texte": text, "le": c["le"] }));
                let o = self.o.up();
                o.emit(json!({ "type": "reseau_message", "de": from, "nom": name, "texte": text }));
                // Lecture à voix haute : réglage par ami (lireMessages), jamais en « ne pas déranger ».
                let read_aloud = js::gb(&self.state.borrow()["lireMessages"], &from.to_string());
                if read_aloud && self.presence()["etat"] != "ne_pas_deranger" {
                    o.emit(json!({ "type": "say", "text": format!("Message de {name} : {text}") }));
                }
            }
            // Accusé de lecture : les identifiants sont ceux que le service a donnés à mes envois.
            "lu" => {
                let ids: Vec<i64> = js::arr(&c["ids"]).iter().filter_map(|v| v.as_i64()).collect();
                let file = self.messages_file(from);
                let mut list: Vec<J> = js::read_json(&file).map(|v| js::arr(&v)).unwrap_or_default();
                for m in list.iter_mut().filter(|m| js::gb(m, "moi") && ids.contains(&(js::gn(m, "id").unwrap_or(0.0) as i64))) {
                    m["luParAmi"] = json!(true);
                }
                let _ = js::write_json(&file, &json!(list));
            }
            // Instantané d'une liste qu'il me partage (lecture seule chez moi).
            "liste" => {
                let id = safe_name(&js::gs(it, "ref"));
                let d = dir.join("listes");
                let _ = std::fs::create_dir_all(&d);
                let _ = js::write_json(&d.join(format!("{id}.json")), &c);
            }
            // Modification d'une de mes listes par un ami : seulement si elle lui est partagée en écriture.
            "liste-op" => {
                let list_id = js::gs(&c, "liste");
                let op = js::gs(&c, "op");
                let granted = js::arr(&f["partagesDonnes"]).iter().any(|s| js::gs(s, "type") == "liste" && js::gs(s, "ref") == list_id && js::gs(s, "rights") == "ecriture");
                if !granted || !FRIEND_LIST_OPS.contains(&op.as_str()) {
                    return Err(format!("{name} n'a pas le droit de modifier cette liste"));
                }
                let mut args = c["args"].clone();
                if !args.is_object() {
                    args = json!({});
                }
                args["liste"] = json!(list_id);
                let r = self.o.up().collections.dispatch(&op, &args, &format!("ami:{name}")).await;
                self.o.up().record("reseau_liste_modifiee", json!({ "par": name, "liste": list_id, "op": op, "ok": r["ok"] }));
            }
            // Morceau d'un fichier d'un dossier partagé ; rangé, jamais exécuté.
            "fichier" => {
                let folder = safe_name(&js::gs(&c, "dossier"));
                let rel: Vec<String> = js::gs(&c, "chemin").split(['/', '\\']).filter(|p| !p.is_empty() && *p != "..").map(safe_name).collect();
                if rel.is_empty() {
                    return Err("chemin de fichier invalide".into());
                }
                let mut dst = dir.join("dossiers").join(&folder);
                for p in &rel {
                    dst = dst.join(p);
                }
                if let Some(p) = dst.parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                let part = b64().decode(js::gs(&c, "data")).map_err(|e| e.to_string())?;
                let first = js::gn(&c, "partie").unwrap_or(0.0) as i64 == 0;
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().create(true).write(true).append(!first).truncate(first).open(&dst).map_err(|e| e.to_string())?;
                file.write_all(&part).map_err(|e| e.to_string())?;
                if js::gb(&c, "fin") {
                    self.o.up().emit(json!({ "type": "reseau_fichier", "de": from, "nom": name, "dossier": folder, "chemin": rel.join("/") }));
                }
            }
            other => return Err(format!("type d'envoi inconnu : {other}")),
        }
        Ok(())
    }

    // --- Partages ---------------------------------------------------------------------------------------------------

    /// Partage (ou retire) presence, une liste ou un dossier avec un ami. La liste part aussitôt.
    pub async fn share(&self, a: &J) -> J {
        let to = js::gn(a, "ami").unwrap_or(0.0) as i64;
        let (kind, r#ref, on) = (js::gs(a, "type"), js::gs(a, "ref"), js::gb(a, "actif"));
        let label = js::gs(a, "label");
        let body = json!({ "ami": to, "type": kind, "ref": r#ref, "label": label, "droits": js::gs(a, "droits"), "actif": on });
        if let Err(e) = self.call("POST", "partage", Some(body)).await {
            return json!({ "ok": false, "error": e });
        }
        self.o.up().record("reseau_partage", json!({ "ami": to, "type": kind, "ref": r#ref, "actif": on, "droits": a["droits"] }));
        let _ = self.refresh_friends().await;
        if on && kind == "liste" {
            self.forget_pushed(to, &r#ref);
            self.push_shared_lists().await;
        }
        json!({ "ok": true, "etat": self.view() })
    }

    fn forget_pushed(&self, to: i64, list: &str) {
        if let Some(o) = self.state.borrow_mut()["envoyees"].as_object_mut() {
            o.remove(&format!("{to}:{list}"));
        }
    }

    /// Renvoie l'instantané de chaque liste partagée qui a changé depuis son dernier envoi. Nombre d'envois.
    async fn push_shared_lists(&self) -> usize {
        let targets: Vec<(i64, String)> = self
            .friends
            .borrow()
            .iter()
            .filter(|f| js::gs(f, "statut") == "accepted")
            .flat_map(|f| {
                let id = js::gn(&f["ami"], "id").unwrap_or(0.0) as i64;
                js::arr(&f["partagesDonnes"]).into_iter().filter(|s| js::gs(s, "type") == "liste").map(move |s| (id, js::gs(&s, "ref")))
            })
            .collect();
        let mut sent = 0;
        for (to, list) in targets {
            let Some(l) = self.o.up().collections.get(&list) else { continue };
            let snapshot = super::collections::schema::public_list(&l, true);
            let h = hash(&snapshot);
            let key = format!("{to}:{list}");
            if js::gs(&self.state.borrow()["envoyees"], &key) == h {
                continue;
            }
            let rights = self.friend(to).map(|f| js::arr(&f["partagesDonnes"]).iter().find(|s| js::gs(s, "ref") == list).map(|s| js::gs(s, "rights")).unwrap_or_default()).unwrap_or_default();
            if self.send_item(to, "liste", &list, &json!({ "liste": snapshot, "droits": rights, "le": js::iso_now() })).await.is_ok() {
                if !self.state.borrow()["envoyees"].is_object() {
                    self.state.borrow_mut()["envoyees"] = json!({});
                }
                self.state.borrow_mut()["envoyees"][&key] = json!(h);
                self.save();
                sent += 1;
            }
        }
        sent
    }

    /// Listes que des amis me partagent (dernier instantané reçu).
    pub fn received_lists(&self) -> J {
        let mut out = vec![];
        for f in self.friends.borrow().iter().filter(|f| js::gs(f, "statut") == "accepted") {
            let id = js::gn(&f["ami"], "id").unwrap_or(0.0) as i64;
            let d = self.friend_dir(id).join("listes");
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if let Some(v) = js::read_json(&e.path()) {
                    out.push(json!({ "ami": id, "nom": f["ami"]["nom"], "liste": v["liste"], "droits": v["droits"], "le": v["le"] }));
                }
            }
        }
        json!(out)
    }

    /// Modification d'une liste qu'un ami me partage en écriture : envoyée à son Jarvis, qui l'applique et renvoie la liste.
    pub async fn edit_friend_list(&self, a: &J) -> J {
        let to = js::gn(a, "ami").unwrap_or(0.0) as i64;
        let op = js::gs(a, "op");
        if !FRIEND_LIST_OPS.contains(&op.as_str()) {
            return json!({ "ok": false, "error": "opération non permise sur une liste d'ami" });
        }
        let list = js::gs(a, "liste");
        match self.send_item(to, "liste-op", &list, &json!({ "liste": list, "op": op, "args": a["args"] })).await {
            Ok(_) => {
                self.fast(FAST_FOR_MS);
                json!({ "ok": true })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    /// Envoie une copie d'un dossier (25 Mo au plus) à un ami, qui la reçoit en lecture seule.
    pub async fn send_folder(&self, to: i64, path: &str) -> J {
        let root = PathBuf::from(path.trim().trim_matches('"'));
        if !root.is_dir() {
            return json!({ "ok": false, "error": "dossier introuvable" });
        }
        let mut files = vec![];
        fn walk(d: &Path, base: &Path, out: &mut Vec<(String, PathBuf, u64)>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else if let Ok(m) = e.metadata() {
                    out.push((p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/"), p.clone(), m.len()));
                }
            }
        }
        walk(&root, &root, &mut files);
        let total: u64 = files.iter().map(|f| f.2).sum();
        if total > FOLDER_MAX {
            return json!({ "ok": false, "error": format!("dossier trop gros ({} Mo, 25 Mo au plus) : partage plutôt un lien OneDrive ou Drive", total / 1_048_576) });
        }
        let folder = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dossier".into());
        for (rel, p, _) in &files {
            let Ok(bytes) = std::fs::read(p) else { continue };
            let parts: Vec<&[u8]> = if bytes.is_empty() { vec![&[][..]] } else { bytes.chunks(CHUNK).collect() };
            let n = parts.len();
            for (i, part) in parts.into_iter().enumerate() {
                let c = json!({ "dossier": folder, "chemin": rel, "partie": i, "fin": i + 1 == n, "data": b64().encode(part) });
                if let Err(e) = self.send_item(to, "fichier", &folder, &c).await {
                    return json!({ "ok": false, "error": e });
                }
            }
        }
        self.o.up().record("reseau_dossier_envoye", json!({ "ami": to, "dossier": folder, "fichiers": files.len(), "octets": total }));
        json!({ "ok": true, "fichiers": files.len(), "octets": total })
    }

    // --- Amis -------------------------------------------------------------------------------------------------------

    pub async fn request(&self, email: &str) -> J {
        match self.call("POST", "ami-demander", Some(json!({ "email": email.trim() }))).await {
            Ok(r) => {
                self.o.up().record("reseau_demande_envoyee", json!({ "a": email, "statut": r["statut"] }));
                let _ = self.refresh_friends().await;
                json!({ "ok": true, "statut": r["statut"], "etat": self.view() })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    pub async fn answer(&self, id: i64, answer: &str) -> J {
        match self.call("POST", "ami-repondre", Some(json!({ "id": id, "reponse": answer }))).await {
            Ok(_) => {
                self.o.up().record("reseau_demande_reponse", json!({ "id": id, "reponse": answer }));
                let _ = self.refresh_friends().await;
                json!({ "ok": true, "etat": self.view() })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    pub async fn remove(&self, id: i64) -> J {
        match self.call("POST", "ami-retirer", Some(json!({ "id": id }))).await {
            Ok(_) => {
                self.o.up().record("reseau_ami_retire", json!({ "id": id }));
                let _ = self.refresh_friends().await;
                json!({ "ok": true, "etat": self.view() })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    pub fn set_option(&self, a: &J) -> J {
        let key = js::gs(a, "cle");
        match key.as_str() {
            "nePasDeranger" => self.state.borrow_mut()["nePasDeranger"] = json!(js::gb(a, "valeur")),
            "lireMessages" => {
                if !self.state.borrow()["lireMessages"].is_object() {
                    self.state.borrow_mut()["lireMessages"] = json!({});
                }
                self.state.borrow_mut()["lireMessages"][js::gs(a, "ami")] = json!(js::gb(a, "valeur"));
            }
            _ => return json!({ "ok": false, "error": "réglage inconnu" }),
        }
        self.save();
        self.last_presence.set(0);
        json!({ "ok": true, "etat": self.view() })
    }

    /// État pour la card Amis et l'administration (jamais de clé).
    pub fn view(&self) -> J {
        let st = self.state.borrow();
        let friends: Vec<J> = self
            .friends
            .borrow()
            .iter()
            .map(|f| {
                let mut f = f.clone();
                let id = js::gn(&f["ami"], "id").unwrap_or(0.0) as i64;
                f["empreinte"] = json!(fingerprint(&js::gs(&f["ami"], "cle")));
                let msgs = js::arr(&self.messages(id));
                f["nonLus"] = json!(msgs.iter().filter(|m| !js::gb(m, "moi") && !js::gb(m, "lu")).count());
                f["dernier"] = msgs.last().cloned().unwrap_or(J::Null);
                f["lireMessages"] = json!(js::gb(&st["lireMessages"], &id.to_string()));
                if let Some(o) = f["ami"].as_object_mut() {
                    o.remove("cle");
                }
                f
            })
            .collect();
        json!({
            "ok": true,
            "relie": self.connected(),
            "serveur": st["serveur"],
            "email": st["email"],
            "compte": { "id": st["compte"]["id"], "nom": st["compte"]["nom"] },
            "empreinte": st["compte"]["cle"].as_str().map(fingerprint),
            "amis": friends,
            "listesRecues": self.received_lists(),
            "nePasDeranger": js::gb(&st, "nePasDeranger"),
            "codeEnAttente": self.pending_email.borrow().is_some(),
            "erreur": Some(self.last_error.borrow().clone()).filter(|e| !e.is_empty()),
            "presence": self.presence()["etat"],
        })
    }

    /// Adresse du service proposée par défaut : celle du déploiement sur ce PC (web-reseau/deploy.ps1), s'il y en a un.
    pub fn suggested_server(&self) -> String {
        let s = self.s("serveur");
        if !s.is_empty() {
            return s;
        }
        let home = js::env("USERPROFILE").unwrap_or_default();
        std::fs::read_to_string(Path::new(&home).join(".jarvis-nomade").join("reseau-url.txt")).map(|t| t.trim().to_string()).unwrap_or_default()
    }

    /// Ouvre la card Amis (n° 124) ; relève rapide tant qu'elle est utilisée.
    pub fn show(&self) {
        self.fast(FAST_FOR_MS);
        let mut v = self.view();
        v["title"] = json!("Amis");
        v["serveurPropose"] = json!(self.suggested_server());
        self.o.up().show_widget("amis_view", v);
    }

    /// Ami accepté dont le nom commence par `spoken` (commande vocale « dis à Léa que… »), sans accents ni casse.
    pub fn friend_by_name(&self, spoken: &str) -> Option<(i64, String)> {
        let want = super::router::fold(spoken);
        self.friends.borrow().iter().filter(|f| js::gs(f, "statut") == "accepted").find_map(|f| {
            let name = js::gs(&f["ami"], "nom");
            let n = super::router::fold(&name);
            (!want.is_empty() && (n == want || n.split_whitespace().next() == Some(want.as_str()))).then(|| (js::gn(&f["ami"], "id").unwrap_or(0.0) as i64, name))
        })
    }

    /// Point d'entrée de la card et de l'administration (méthode « reseau » de la passerelle).
    pub async fn dispatch(&self, op: &str, a: &J) -> J {
        let id = js::gn(a, "ami").unwrap_or(0.0) as i64;
        match op {
            "" | "etat" => {
                let mut v = self.view();
                v["serveurPropose"] = json!(self.suggested_server());
                v
            }
            "ouvrir" => {
                self.show();
                json!({ "ok": true })
            }
            "rapide" => {
                self.fast(FAST_FOR_MS);
                if self.connected() && !self.busy.get() {
                    self.cycle().await;
                }
                self.view()
            }
            "inscription" => self.register(&js::gs(a, "serveur"), &js::gs(a, "email"), &js::gs(a, "nom")).await,
            "verification" => self.verify(&js::gs(a, "code")).await,
            "oublier" => self.forget(false).await,
            "supprimerCompte" => self.forget(true).await,
            "demander" => self.request(&js::gs(a, "email")).await,
            "repondre" => self.answer(js::gn(a, "id").unwrap_or(0.0) as i64, &js::gs(a, "reponse")).await,
            "retirer" => self.remove(js::gn(a, "id").unwrap_or(0.0) as i64).await,
            "partager" => self.share(a).await,
            "messages" => json!({ "ok": true, "messages": self.messages(id) }),
            "envoyer" => self.send_message(id, &js::gs(a, "texte")).await,
            "lu" => self.mark_read(id).await,
            "modifierListeAmi" => self.edit_friend_list(a).await,
            "envoyerDossier" => self.send_folder(id, &js::gs(a, "chemin")).await,
            "ouvrirRecus" => {
                let d = self.friend_dir(id);
                let _ = std::fs::create_dir_all(&d);
                crate::proc::detached("explorer.exe", &[&d.to_string_lossy()]);
                json!({ "ok": true })
            }
            "reglage" => self.set_option(a),
            "mesListes" => {
                let lists: Vec<J> = self.o.up().collections.all().iter().map(|l| json!({ "id": l["id"], "nom": l["nom"] })).collect();
                json!({ "ok": true, "listes": lists })
            }
            _ => json!({ "ok": false, "error": format!("opération inconnue : {op}") }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chiffre_pour_l_ami_et_lui_seul() {
        let (alice, bob, eve) = (SecretKey::generate(&mut OsRng), SecretKey::generate(&mut OsRng), SecretKey::generate(&mut OsRng));
        let pk = |k: &SecretKey| b64().encode(k.public_key().as_bytes());
        let msg = json!({ "texte": "Coucou papa", "le": "2026-10-28" });
        let (nonce, ct) = seal(&alice, &pk(&bob), &msg).unwrap();
        assert!(!String::from_utf8_lossy(&b64().decode(&ct).unwrap()).contains("Coucou"));
        assert_eq!(open(&bob, &pk(&alice), &nonce, &ct).unwrap(), msg);
        // Une tierce clé (ou un mauvais expéditeur) ne lit rien.
        assert!(open(&eve, &pk(&alice), &nonce, &ct).is_err());
        assert!(open(&bob, &pk(&eve), &nonce, &ct).is_err());
    }

    #[test]
    fn noms_de_fichiers_sans_remontee() {
        assert_eq!(safe_name(".."), "sans-nom");
        assert_eq!(safe_name("a/b:c"), "a_b_c");
        assert_eq!(fingerprint("abc").split('-').count(), 4);
    }
}
