// Jarvis Nomade : une partie des données de Jarvis consultable hors du PC (site hébergé chez Hostinger,
// web-nomade/). Jarvis n'ouvre aucun port : il appelle l'API du site (HTTPS sortant), envoie les sources
// cochées dans l'administration (par différence : seul ce qui a changé part) et reprend ce qui a été
// modifié ou déposé sur le site, en passant par les services habituels (listes, notes, rappels).
// Réglages et secrets dans data/nomade.json (mot de passe et clé d'appareil chiffrés par DPAPI) ;
// état de la synchronisation et dépôts reçus dans data/nomade/.

pub mod demande;
pub mod sources;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use serde_json::{json, Map, Value};

use super::intent::Intent;
use super::skill_intents::it;
use super::{OrchRef, Orchestrator, Up};
use crate::js::{self, J, Timer};
use crate::secret_box::SecretBox;

/// Lots envoyés au site : au plus autant d'enregistrements, et à peu près autant d'octets, par requête
/// (le site refuse au-delà de 4 Mo).
const BATCH: usize = 250;
const BATCH_BYTES: usize = 1_500_000;
const MAX_CONFLICTS: usize = 30;
const MAX_DEPOTS: usize = 100;
const DEFAULT_INTERVAL_S: u64 = 45;

fn defaults() -> J {
    json!({ "url": "", "login": "", "password": "", "key": "", "sources": [], "enabled": true, "intervalS": DEFAULT_INTERVAL_S })
}

/// Adresse du site normalisée (https obligatoire, sans « / » final).
pub fn clean_url(raw: &str) -> Result<String, String> {
    let u = raw.trim().trim_end_matches('/').to_string();
    let ok = u.starts_with("https://") && u.len() > 12 && !u.contains(char::is_whitespace) && !u.contains('?') && !u.contains('#');
    if ok { Ok(u) } else { Err("L'adresse du site doit commencer par https:// (ex. https://jarvis.mon-domaine.fr).".into()) }
}

pub fn valid_login(login: &str) -> bool {
    let n = login.chars().count();
    (3..=80).contains(&n) && login.chars().all(|c| c.is_alphanumeric() || "._@-".contains(c))
}

/// Ajouts, modifications et suppressions à envoyer pour une source, d'après ce qui a déjà été envoyé.
pub fn diff(pushed: &Map<String, Value>, current: &[(String, J)]) -> (Vec<(String, J, String)>, Vec<String>) {
    let mut seen = HashSet::new();
    let mut upserts = vec![];
    for (rid, data) in current {
        if rid.is_empty() || !seen.insert(rid.clone()) {
            continue;
        }
        let h = sources::hash(data);
        if pushed.get(rid).and_then(|v| v.as_str()) != Some(h.as_str()) {
            upserts.push((rid.clone(), data.clone(), h));
        }
    }
    let deletes = pushed.keys().filter(|k| !seen.contains(*k)).cloned().collect();
    (upserts, deletes)
}

/// Champs modifiés des deux côtés avec des valeurs différentes (le site l'emporte, Jarvis garde la trace).
pub fn conflicting_fields(local: &J, patch: &J) -> Vec<J> {
    let Some(p) = patch.as_object() else { return vec![] };
    p.iter()
        .filter(|(k, _)| !k.starts_with('_') || *k == "_statut")
        .filter_map(|(k, v)| {
            let l = &local[k.as_str()];
            (!l.is_null() && js::s(l) != js::s(v)).then(|| json!({ "champ": k, "jarvis": l, "site": v }))
        })
        .collect()
}

/// Une modification refusée garde ce que le site avait envoyé (`site`) : elle reste seule, on peut la réessayer.
fn conflict_key(c: &J) -> Option<String> {
    c["site"].is_null().then(|| format!("{}|{}|{}|{}", js::s(&c["source"]), js::s(if c["titre"].is_null() { &c["id"] } else { &c["titre"] }), js::s(&c["erreur"]), c["champs"]))
}

/// Écarts « à vérifier » (du plus récent au plus ancien) : les mêmes regroupés en un seul, avec leur nombre (`fois`).
pub fn merge_conflicts(list: Vec<J>) -> Vec<J> {
    let mut out: Vec<J> = vec![];
    for c in list {
        match conflict_key(&c).and_then(|k| out.iter_mut().find(|o| conflict_key(o).as_deref() == Some(k.as_str()))) {
            Some(first) => first["fois"] = json!(first["fois"].as_u64().unwrap_or(1) + c["fois"].as_u64().unwrap_or(1)),
            None => out.push(c),
        }
    }
    out.truncate(MAX_CONFLICTS);
    out
}

/// Valeurs sans les champs de type lien ou image de la liste (repli quand l'un d'eux est refusé).
fn without_links(list: Option<&J>, values: &J) -> J {
    let mut out = values.clone();
    let (Some(list), Some(m)) = (list, out.as_object_mut()) else { return out };
    for f in js::arr(&list["champs"]) {
        if ["lien", "image"].contains(&js::gs(&f, "type").as_str()) {
            m.remove(&js::gs(&f, "cle"));
        }
    }
    out
}

/// Valeurs d'un enregistrement venu du site, sans les champs réservés (`_t`, `_statut`…).
fn plain_values(v: &J) -> (J, String) {
    let mut out = Map::new();
    if let Some(o) = v.as_object() {
        for (k, x) in o {
            if !k.starts_with('_') {
                out.insert(k.clone(), x.clone());
            }
        }
    }
    (Value::Object(out), js::gs(v, "_statut"))
}

pub struct Nomade {
    me: Weak<Nomade>,
    o: OrchRef,
    file: PathBuf,
    dir: PathBuf,
    secret_box: Option<Rc<SecretBox>>,
    log: Rc<dyn Fn(String)>,
    pub config: RefCell<J>,
    /// { seq, pushed: { source: { id: empreinte } }, meta: { source: empreinte }, conflicts: [], tempDeletes: [] }
    sync: RefCell<J>,
    depots: RefCell<Vec<J>>,
    busy: Cell<bool>,
    again: Cell<bool>,
    timer: Timer,
    poke_timer: Timer,
    listening: Cell<bool>,
    last_sync: Cell<i64>,
    last_error: RefCell<String>,
    last_summary: RefCell<J>,
    /// Listes des invités à relire (changement vu sur le site, partage modifié, démarrage).
    friends_dirty: Cell<bool>,
    /// Dernière comparaison du site déployé avec le dossier web-nomade de Jarvis.
    site: RefCell<J>,
}

impl Nomade {
    pub fn new(o: OrchRef, file: PathBuf, dir: PathBuf, secret_box: Option<Rc<SecretBox>>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let n = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            o,
            sync: RefCell::new(js::read_json(&dir.join("etat.json")).unwrap_or(json!({}))),
            depots: RefCell::new(js::read_json(&dir.join("depots.json")).map(|v| js::arr(&v)).unwrap_or_default()),
            file,
            dir,
            secret_box,
            log,
            config: RefCell::new(defaults()),
            busy: Cell::new(false),
            again: Cell::new(false),
            timer: Timer::new(),
            poke_timer: Timer::new(),
            listening: Cell::new(false),
            last_sync: Cell::new(0),
            last_error: RefCell::new(String::new()),
            last_summary: RefCell::new(Value::Null),
            friends_dirty: Cell::new(true),
            site: RefCell::new(Value::Null),
        });
        *n.config.borrow_mut() = n.load();
        // Écarts notés avant leur regroupement (4.88.0) : regroupés, les refus nommés d'après la fiche du site.
        if let Some(list) = n.sync.borrow_mut()["conflicts"].as_array_mut() {
            for c in list.iter_mut().filter(|c| c["titre"].is_null() && c["site"]["_t"].is_string()) {
                c["titre"] = c["site"]["_t"].clone();
            }
            *list = merge_conflicts(std::mem::take(list));
        }
        n
    }

    // --- Réglages -------------------------------------------------------------------------------

    fn load(&self) -> J {
        let raw = js::read_json(&self.file).unwrap_or(json!({}));
        let mut cfg = js::spread(&defaults(), &raw);
        for (clear, enc) in [("password", "passwordEnc"), ("key", "keyEnc")] {
            cfg[clear] = json!(match (&self.secret_box, raw[enc].as_str()) {
                (Some(b), Some(e)) => b.decrypt(e).unwrap_or_default(),
                _ => String::new(),
            });
            cfg.as_object_mut().unwrap().remove(enc);
        }
        cfg
    }

    /// Les secrets ne sont écrits que chiffrés : sans chiffrement disponible, ils restent en mémoire.
    fn save(&self) {
        let stored = Self::stored_form(&self.config.borrow(), |s| self.secret_box.as_ref().and_then(|b| b.encrypt(s).ok()));
        let _ = js::write_json(&self.file, &stored);
    }

    pub fn stored_form(cfg: &J, encrypt: impl Fn(&str) -> Option<String>) -> J {
        let mut stored = cfg.clone();
        for (clear, enc) in [("password", "passwordEnc"), ("key", "keyEnc")] {
            let v = js::gs(&stored, clear);
            stored.as_object_mut().unwrap().remove(clear);
            if let Some(e) = Some(v).filter(|v| !v.is_empty()).and_then(|v| encrypt(&v)) {
                stored[enc] = json!(e);
            }
        }
        stored
    }

    fn c(&self, k: &str) -> J {
        self.config.borrow()[k].clone()
    }

    fn configured(&self) -> bool {
        js::truthy(&self.c("key")) && js::truthy(&self.c("url"))
    }

    fn shared(&self) -> Vec<String> {
        js::str_list(&self.c("sources"))
    }

    fn save_sync(&self) {
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = js::write_json_atomic(&self.dir.join("etat.json"), &self.sync.borrow());
    }

    fn save_depots(&self) {
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = js::write_json(&self.dir.join("depots.json"), &Value::Array(self.depots.borrow().clone()));
    }

    /// Fichier déposé par web-nomade/deploy.ps1 (code d'installation, adresse du site).
    fn hint(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).map(|s| s.trim().to_string()).unwrap_or_default()
    }

    pub fn state(&self) -> J {
        let o = self.o.up();
        let shared = self.shared();
        let catalog = sources::catalog(&o);
        let known: HashSet<String> = catalog.iter().map(|s| s.id.clone()).collect();
        let sync = self.sync.borrow();
        let url = js::gs(&self.config.borrow(), "url");
        json!({
            "configured": self.configured(), "url": url, "urlHint": self.hint("site-url.txt"), "login": self.c("login"),
            "hasPassword": js::truthy(&self.c("password")), "encrypted": self.secret_box.is_some(),
            "installAvailable": !self.hint("install-token.txt").is_empty(),
            "enabled": self.c("enabled") != json!(false), "intervalS": self.interval_s(),
            "sources": catalog.iter().map(|s| {
                let mut p = s.public(shared.contains(&s.id));
                p["sent"] = json!(sync["pushed"][s.id.as_str()].as_object().map(|m| m.len()).unwrap_or(0));
                p
            }).collect::<Vec<_>>(),
            "missing": shared.iter().filter(|s| !known.contains(*s)).collect::<Vec<_>>(),
            "lastSync": self.last_sync.get(), "lastError": *self.last_error.borrow(), "busy": self.busy.get(),
            "summary": *self.last_summary.borrow(),
            "conflicts": js::arr(&sync["conflicts"]).into_iter().map(|mut c| {
                let src = js::gs(&c, "source");
                c["sourceLabel"] = json!(catalog.iter().find(|s| s.id == src).map(|s| s.label.clone()).unwrap_or(src));
                c["retry"] = json!(!c["site"].is_null());
                c
            }).collect::<Vec<_>>(),
            "depots": Value::Array(self.depots.borrow().clone()),
            "site": *self.site.borrow(),
            "friends": js::read_json(&self.dir.join("amis.json")).map(|a| js::arr(&a["friends"]).iter().map(|f| json!({
                "name": f["name"], "relation": f["relation"], "lists": js::arr(&f["lists"]).iter().map(|l| json!({ "title": l["title"], "items": js::arr(&l["items"]).len() })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()).unwrap_or_default(),
        })
    }

    fn interval_s(&self) -> u64 {
        self.c("intervalS").as_u64().filter(|n| (15..=3600).contains(n)).unwrap_or(DEFAULT_INTERVAL_S)
    }

    // --- Appels au site -------------------------------------------------------------------------

    async fn call(&self, route: &str, query: &str, body: Option<J>) -> Result<J, String> {
        let (url, key) = (js::gs(&self.config.borrow(), "url"), js::gs(&self.config.borrow(), "key"));
        self.call_with(&url, &key, route, query, body).await
    }

    async fn call_with(&self, url: &str, key: &str, route: &str, query: &str, body: Option<J>) -> Result<J, String> {
        if url.is_empty() {
            return Err("Adresse du site non réglée.".into());
        }
        let full = format!("{url}/api/?r={route}{query}");
        let http = crate::net::http();
        let mut req = match &body {
            Some(b) => http.post(&full).json(b),
            None => http.get(&full),
        };
        req = req.header("X-Jarvis-Version", env!("CARGO_PKG_VERSION")).timeout(std::time::Duration::from_secs(40));
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
        let res = req.send().await.map_err(|e| format!("site injoignable ({})", e.without_url()))?;
        let status = res.status().as_u16();
        let data: Option<J> = res.json().await.ok();
        match data {
            Some(d) if js::gb(&d, "ok") => Ok(d),
            Some(d) if js::truthy(&d["error"]) => Err(js::gs(&d, "error")),
            _ => Err(format!("réponse inattendue du site (HTTP {status})")),
        }
    }

    // --- Administration -------------------------------------------------------------------------

    /// Relie Jarvis au site : identifiant et mot de passe du site (création du compte sur un site vierge,
    /// avec le code d'installation laissé par deploy.ps1). Une nouvelle clé d'appareil est obtenue.
    pub async fn pair(&self, url: &str, login: &str, password: &str) -> J {
        let url = match clean_url(if url.trim().is_empty() { self.hint("site-url.txt") } else { url.to_string() }.as_str()) {
            Ok(u) => u,
            Err(e) => return json!({ "ok": false, "error": e }),
        };
        let login = login.trim().to_string();
        let password = if password.is_empty() && login == js::s(&self.c("login")) { js::s(&self.c("password")) } else { password.to_string() };
        if !valid_login(&login) || password.chars().count() < 10 {
            return json!({ "ok": false, "error": "Identifiant de 3 caractères au moins (lettres, chiffres, . _ @ -) et mot de passe de 10 caractères au moins." });
        }
        let body = json!({ "login": login, "password": password, "install": self.hint("install-token.txt"), "device": format!("Jarvis {}", js::env("COMPUTERNAME").unwrap_or_default()).trim() });
        let res = match self.call_with(&url, "", "pair", "", Some(body)).await {
            Ok(r) => r,
            Err(e) => return json!({ "ok": false, "error": e }),
        };
        let same_site = js::gs(&self.config.borrow(), "url") == url;
        {
            let mut cfg = self.config.borrow_mut();
            cfg["url"] = json!(url);
            cfg["login"] = json!(login);
            cfg["password"] = json!(password);
            cfg["key"] = res["key"].clone();
        }
        if !same_site {
            *self.sync.borrow_mut() = json!({});
            self.save_sync();
        }
        self.save();
        if js::gb(&res, "created") {
            let _ = std::fs::remove_file(self.dir.join("install-token.txt"));
        }
        self.restart();
        js::spread(&json!({ "ok": true, "created": res["created"] }), &self.state())
    }

    /// Nouvel identifiant ou mot de passe du site (après le code de validation de l'administration).
    pub async fn change_account(&self, login: &str, password: &str, revoke_sessions: bool) -> J {
        if !self.configured() {
            return json!({ "ok": false, "error": "Jarvis n'est pas encore relié au site." });
        }
        let login = login.trim().to_string();
        let password = if password.is_empty() { js::s(&self.c("password")) } else { password.to_string() };
        if !valid_login(&login) || password.chars().count() < 10 {
            return json!({ "ok": false, "error": "Identifiant de 3 caractères au moins (lettres, chiffres, . _ @ -) et mot de passe de 10 caractères au moins." });
        }
        if let Err(e) = self.call("account", "", Some(json!({ "login": login, "password": password, "revokeSessions": revoke_sessions }))).await {
            return json!({ "ok": false, "error": e });
        }
        self.config.borrow_mut()["login"] = json!(login);
        self.config.borrow_mut()["password"] = json!(password);
        self.save();
        js::spread(&json!({ "ok": true }), &self.state())
    }

    /// Partager une source (envoi immédiat). Le retrait passe par `drop_source` (code de validation).
    pub fn share(&self, id: &str) -> J {
        let o = self.o.up();
        if !sources::catalog(&o).iter().any(|s| s.id == id) {
            return json!({ "ok": false, "error": "Source inconnue." });
        }
        let mut list = self.shared();
        if !list.iter().any(|s| s == id) {
            list.push(id.to_string());
            self.config.borrow_mut()["sources"] = json!(list);
            self.save();
            self.poke(500);
        }
        js::spread(&json!({ "ok": true }), &self.state())
    }

    /// Source retirée : effacée du site, puis oubliée par Jarvis.
    pub async fn drop_source(&self, id: &str) -> J {
        if self.configured() {
            if let Err(e) = self.call("drop", "", Some(json!({ "source": id }))).await {
                return json!({ "ok": false, "error": format!("Le site n'a pas effacé ces données : {e}") });
            }
        }
        if self.configured() {
            let _ = self.call("gc", "", Some(json!({}))).await;
        }
        let list: Vec<String> = self.shared().into_iter().filter(|s| s != id).collect();
        self.config.borrow_mut()["sources"] = json!(list);
        self.save();
        {
            let mut sync = self.sync.borrow_mut();
            for k in ["pushed", "meta"] {
                if let Some(m) = sync[k].as_object_mut() {
                    m.remove(id);
                }
            }
        }
        self.save_sync();
        js::spread(&json!({ "ok": true }), &self.state())
    }

    pub fn set_options(&self, opts: &J) -> J {
        if let Some(b) = opts["enabled"].as_bool() {
            self.config.borrow_mut()["enabled"] = json!(b);
        }
        if let Some(n) = opts["intervalS"].as_u64() {
            self.config.borrow_mut()["intervalS"] = json!(n.clamp(15, 3600));
        }
        self.save();
        self.restart();
        js::spread(&json!({ "ok": true }), &self.state())
    }

    pub async fn sessions(&self) -> J {
        match self.call("sessions", "", None).await {
            Ok(r) => r,
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    pub async fn revoke(&self, id: i64) -> J {
        match self.call("revoke", "", Some(json!({ "id": id }))).await {
            Ok(_) => self.sessions().await,
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    /// Jarvis oublie le site (rien n'est effacé en ligne) ; les listes des invités quittent sa mémoire.
    pub fn forget(&self) -> J {
        self.timer.clear();
        let o = self.o.up();
        if let Some(a) = js::read_json(&self.dir.join("amis.json")) {
            for pin in js::str_list(&a["pins"]) {
                o.memory.forget_pinned(&pin);
            }
            let _ = std::fs::remove_file(self.dir.join("amis.json"));
            o.publish_memory();
        }
        *self.config.borrow_mut() = defaults();
        *self.sync.borrow_mut() = json!({});
        let _ = std::fs::remove_file(&self.file);
        self.save_sync();
        js::spread(&json!({ "ok": true }), &self.state())
    }

    pub async fn sync_now(&self) -> J {
        let Some(me) = self.me.upgrade() else { return json!({ "ok": false }) };
        match me.cycle().await {
            Ok(summary) => js::spread(&json!({ "ok": true, "result": summary }), &self.state()),
            Err(e) => js::spread(&json!({ "ok": false, "error": e }), &self.state()),
        }
    }

    // --- Boucle ---------------------------------------------------------------------------------

    pub fn start(&self) {
        if !self.listening.get() {
            self.listening.set(true);
            let me = self.me.clone();
            self.o.up().emit_listeners.borrow_mut().push(Rc::new(move |e: &J| {
                if js::gs(e, "type") == "collections_changed" {
                    if let Some(s) = me.upgrade() {
                        s.poke(4000);
                    }
                }
            }));
        }
        if !self.configured() || self.c("enabled") == json!(false) {
            return;
        }
        let me = self.me.clone();
        self.timer.every(self.interval_s() * 1000, move || {
            if let Some(s) = me.upgrade() {
                js::spawn(async move {
                    let _ = s.cycle().await;
                });
            }
        });
        self.poke(8000);
    }

    pub fn stop(&self) {
        self.timer.clear();
        self.poke_timer.clear();
    }

    fn restart(&self) {
        self.stop();
        self.start();
    }

    /// Synchronisation prochaine (modification locale, source cochée) ; les appels rapprochés n'en font qu'une.
    pub fn poke(&self, ms: u64) {
        if !self.configured() || self.c("enabled") == json!(false) {
            return;
        }
        let me = self.me.clone();
        self.poke_timer.set(ms, move || {
            if let Some(s) = me.upgrade() {
                js::spawn(async move {
                    let _ = s.cycle().await;
                });
            }
        });
    }

    async fn cycle(self: Rc<Self>) -> Result<J, String> {
        if !self.configured() {
            return Err("Jarvis n'est pas relié au site.".into());
        }
        if self.busy.get() {
            self.again.set(true);
            return Ok(json!({ "skipped": true }));
        }
        self.busy.set(true);
        let res = self.cycle_inner().await;
        self.busy.set(false);
        match &res {
            Ok(summary) => {
                self.last_sync.set(js::now_ms());
                self.last_error.borrow_mut().clear();
                *self.last_summary.borrow_mut() = summary.clone();
            }
            Err(e) => {
                if *self.last_error.borrow() != *e {
                    (self.log)(format!("Nomade : {e}"));
                }
                *self.last_error.borrow_mut() = e.clone();
            }
        }
        self.o.up().emit(json!({ "type": "nomade", "lastSync": self.last_sync.get(), "error": *self.last_error.borrow(), "depots": self.depots.borrow().len() }));
        if self.again.replace(false) {
            self.poke(1000);
        }
        res
    }

    async fn cycle_inner(&self) -> Result<J, String> {
        let received = self.pull().await?;
        if self.friends_dirty.get() {
            self.fetch_friends().await?;
        }
        if js::now_ms() - js::gn_or(&self.site.borrow(), "checkedAt", 0.0) as i64 > SITE_CHECK_MS {
            let _ = self.check_site().await;
        }
        let (sent, deleted) = self.push().await?;
        self.apply_grants().await;
        Ok(json!({ "recus": received, "envoyes": sent, "supprimes": deleted, "at": js::now_ms() }))
    }

    // --- Lecture : ce qui vient du site ---------------------------------------------------------

    async fn pull(&self) -> Result<usize, String> {
        let mut applied = 0;
        loop {
            let since = self.sync.borrow()["seq"].as_i64().unwrap_or(0);
            let res = self.call("pull", &format!("&since={since}"), None).await?;
            let o = self.o.up();
            let mut handled_temp = HashSet::new();
            for ch in js::arr(&res["changes"]) {
                if js::gs(&ch, "source").starts_with(FRIEND_PREFIX) {
                    self.friends_dirty.set(true);
                    continue;
                }
                let rid = js::gs(&ch, "id");
                // Élément créé sur le site : seule sa version la plus récente (`data`) est reprise, une fois.
                if rid.starts_with("w-") && (js::gs(&ch, "op") != "add" || !handled_temp.insert(rid.clone())) {
                    continue;
                }
                let mut refused = false;
                match self.apply(&o, &ch) {
                    Ok(true) => {
                        applied += 1;
                        o.record("nomade_modification", json!({ "source": ch["source"], "op": ch["op"], "auteur": ch["author"] }));
                    }
                    Ok(false) => {}
                    Err(e) => {
                        refused = true;
                        let site = if ch["patch"].is_null() { ch["data"].clone() } else { ch["patch"].clone() };
                        self.note_conflict(json!({ "source": ch["source"], "id": rid, "op": ch["op"], "titre": site["_t"], "erreur": e, "site": site }));
                    }
                }
                // Élément refusé par Jarvis : il reste sur le site (en attente ⟳) au lieu de disparaître sans trace.
                if rid.starts_with("w-") && !refused {
                    self.push_temp_delete(&js::gs(&ch, "source"), &rid);
                }
            }
            let inbox = js::arr(&res["inbox"]);
            if !inbox.is_empty() {
                self.receive_depots(&o, inbox).await;
            }
            self.sync.borrow_mut()["seq"] = res["seq"].clone();
            self.save_sync();
            if !js::gb(&res, "more") {
                break;
            }
        }
        Ok(applied)
    }

    fn push_temp_delete(&self, source: &str, rid: &str) {
        let mut sync = self.sync.borrow_mut();
        if !sync["tempDeletes"].is_array() {
            sync["tempDeletes"] = json!([]);
        }
        sync["tempDeletes"].as_array_mut().unwrap().push(json!({ "source": source, "id": rid }));
    }

    fn note_conflict(&self, mut c: J) {
        c["at"] = json!(js::now_ms());
        (self.log)(format!("Nomade : modification du site à vérifier ({})", js::slice_chars(&c.to_string(), 300)));
        let mut sync = self.sync.borrow_mut();
        let mut list = js::arr(&sync["conflicts"]);
        list.insert(0, c);
        sync["conflicts"] = Value::Array(merge_conflicts(list));
    }

    /// Écarts « à vérifier » : `reessayer` rejoue une modification refusée, `ignorer` en retire un, `effacer` les retire tous.
    /// Ignorer une modification refusée donne raison à Jarvis : la fiche créée sur le site est retirée, une fiche modifiée
    /// y est renvoyée telle que Jarvis la connaît.
    pub fn conflict(&self, action: &str, at: i64, rid: &str) -> J {
        let list = js::arr(&self.sync.borrow()["conflicts"]);
        let picked: Vec<J> = list.iter().filter(|c| action == "effacer" || (c["at"].as_i64() == Some(at) && js::gs(c, "id") == rid)).cloned().collect();
        if picked.is_empty() {
            return json!({ "ok": false, "error": "Élément introuvable (déjà traité ?)." });
        }
        let kept: Vec<J> = list.into_iter().filter(|c| !picked.contains(c)).collect();
        self.sync.borrow_mut()["conflicts"] = Value::Array(kept);
        let say = match action {
            "reessayer" => self.retry_conflict(&picked[0]),
            "ignorer" | "effacer" => {
                for c in picked.iter().filter(|c| !c["site"].is_null()) {
                    let (source, id) = (js::gs(c, "source"), js::gs(c, "id"));
                    if id.starts_with("w-") {
                        self.push_temp_delete(&source, &id);
                    } else if let Some(m) = self.sync.borrow_mut().pointer_mut(&format!("/pushed/{}", source.replace('~', "~0").replace('/', "~1"))).and_then(|v| v.as_object_mut()) {
                        m.remove(&id); // renvoyée au prochain échange
                    }
                }
                Ok(if action == "effacer" { "Liste à vérifier vidée.".to_string() } else { "Retiré.".to_string() })
            }
            _ => Err("Action inconnue.".to_string()),
        };
        self.save_sync();
        self.poke(1000);
        match say {
            Ok(msg) => js::spread(&json!({ "ok": true, "say": msg }), &self.state()),
            Err(e) => js::spread(&json!({ "ok": false, "error": e }), &self.state()),
        }
    }

    /// Rejoue une modification refusée avec ce que le site avait envoyé (les règles ont pu changer depuis).
    fn retry_conflict(&self, c: &J) -> Result<String, String> {
        if c["site"].is_null() {
            return Err("Rien à réessayer : cette modification a déjà été appliquée.".into());
        }
        let rid = js::gs(c, "id");
        let op = match js::gs(c, "op") {
            op if !op.is_empty() => op,
            _ if rid.starts_with("w-") => "add".to_string(),
            _ => "update".to_string(),
        };
        let ch = json!({ "source": c["source"], "id": rid, "op": op, "data": c["site"], "patch": c["site"] });
        let start = js::now_ms();
        let o = self.o.up();
        let res = self.apply(&o, &ch);
        if let Err(e) = &res {
            self.note_conflict(js::spread(c, &json!({ "erreur": e })));
            return Err(format!("Toujours refusé : {e}"));
        }
        // Ce que l'essai vient de signaler (doublon, champ retiré) est dit tout de suite au lieu d'être gardé.
        let mut noted = String::new();
        {
            let mut sync = self.sync.borrow_mut();
            let list: Vec<J> = js::arr(&sync["conflicts"]).into_iter().filter(|x| {
                let fresh = js::gs(x, "id") == rid && x["at"].as_i64().unwrap_or(0) >= start;
                if fresh && noted.is_empty() {
                    noted = js::gs(x, "erreur");
                }
                !fresh
            }).collect();
            sync["conflicts"] = Value::Array(list);
        }
        if rid.starts_with("w-") {
            self.push_temp_delete(&js::gs(c, "source"), &rid);
        }
        o.record("nomade_modification", json!({ "source": c["source"], "op": op, "reprise": true }));
        Ok(if noted.is_empty() { "Modification appliquée.".to_string() } else { format!("Fait : {noted}") })
    }

    /// Une modification faite sur le site, appliquée par le service du magasin concerné.
    fn apply(&self, o: &Rc<Orchestrator>, ch: &J) -> Result<bool, String> {
        let source = js::gs(ch, "source");
        let rid = js::gs(ch, "id");
        let op = js::gs(ch, "op");
        let patch = if op == "add" { ch["data"].clone() } else { ch["patch"].clone() };
        if op == "add" && patch.is_null() {
            return Ok(false); // ajouté puis supprimé sur le site avant que Jarvis ne le voie
        }
        // Modifié aussi dans Jarvis depuis le dernier envoi : le site l'emporte, l'écart est gardé.
        if op == "update" || op == "status" {
            if let Some(cur) = sources::records(o, &source).and_then(|rows| rows.into_iter().find(|(id, _)| *id == rid)).map(|(_, d)| d) {
                let pushed = self.sync.borrow()["pushed"][source.as_str()][rid.as_str()].as_str().map(String::from);
                let fields = conflicting_fields(&cur, &patch);
                if pushed.is_some_and(|h| h != sources::hash(&cur)) && !fields.is_empty() {
                    self.note_conflict(json!({ "source": source, "id": rid, "titre": cur["_t"], "champs": fields }));
                }
            }
        }
        if let Some(list_id) = source.strip_prefix("col:") {
            let c = &o.collections;
            let (values, statut) = plain_values(&patch);
            match op.as_str() {
                "add" => {
                    let extra = json!({ "statut": statut, "codeBarre": patch["_barcode"] });
                    let r = match c.add_item(list_id, &values, &extra, "nomade") {
                        Ok(r) => r,
                        // Un lien ou une image invalide ne doit pas faire perdre la fiche : ajoutée sans ce champ, écart signalé.
                        Err(e) => {
                            let bare = without_links(c.get(list_id).as_ref(), &values);
                            if bare == values {
                                return Err(e);
                            }
                            let r = c.add_item(list_id, &bare, &extra, "nomade").map_err(|_| e.clone())?;
                            self.note_conflict(json!({ "source": source, "id": rid, "titre": patch["_t"], "erreur": format!("{e} La fiche est ajoutée sans ce champ.") }));
                            r
                        }
                    };
                    if js::gb(&r, "doublon") {
                        self.note_conflict(json!({ "source": source, "id": rid, "titre": patch["_t"], "erreur": "déjà dans la liste : non ajouté" }));
                        return Ok(false);
                    }
                }
                "update" | "status" => {
                    if values.as_object().is_some_and(|m| !m.is_empty()) {
                        c.update_item(list_id, &rid, &values, "nomade")?;
                    }
                    if !statut.is_empty() {
                        c.set_status(list_id, &rid, statut == "possede", &json!({}), "nomade")?;
                    }
                }
                "delete" => {
                    c.delete_item(list_id, &rid, "nomade")?;
                }
                _ => return Ok(false),
            }
            return Ok(true);
        }
        if let Some(list_id) = source.strip_prefix("liste:") {
            let mut list = o.smart_lists.get(list_id).ok_or("Liste introuvable.")?;
            let mut items = js::arr(&list["items"]);
            match op.as_str() {
                "add" => {
                    let text = js::slice_chars(js::gs(&patch, "text").trim(), 300);
                    if text.is_empty() {
                        return Ok(false);
                    }
                    let id = items.iter().filter_map(|i| i["id"].as_i64().or_else(|| js::s(&i["id"]).parse().ok())).max().unwrap_or(0) + 1;
                    items.push(json!({ "id": id, "text": text, "checked": false, "category": js::slice_chars(js::gs(&patch, "category").trim(), 60) }));
                }
                "delete" => items.retain(|i| js::s(&i["id"]) != rid),
                _ => {
                    let item = items.iter_mut().find(|i| js::s(&i["id"]) == rid).ok_or("Élément introuvable dans la liste.")?;
                    if let Some(b) = patch["checked"].as_bool() {
                        item["checked"] = json!(b);
                    }
                    for k in ["text", "category"] {
                        if patch[k].is_string() {
                            item[k] = json!(js::slice_chars(js::gs(&patch, k).trim(), 300));
                        }
                    }
                }
            }
            list["items"] = Value::Array(items);
            if !o.smart_lists.save_list(&mut list) {
                return Err("Je n'ai pas pu enregistrer la liste.".into());
            }
            return Ok(true);
        }
        match (source.as_str(), op.as_str()) {
            ("notes", "add") => {
                o.notes.write_new(&js::gs(&patch, "title"), &js::gs(&patch, "body"), "nomade")?;
            }
            ("rappels", "add") => {
                let at = js::num(&patch["at"]).map(|n| n as i64).filter(|at| *at > 0).ok_or("Date du rappel absente.")?;
                let what = js::slice_chars(js::gs(&patch, "what").trim(), 300);
                if what.is_empty() {
                    return Err("Rappel sans texte.".into());
                }
                o.reminders.add(&what, at, Value::Null);
                o.record("rappel_ajoute", json!({ "texte": what, "origine": "nomade" }));
            }
            ("rappels", "delete") => {
                o.reminders.cancel(rid.parse().map_err(|_| "Rappel inconnu.")?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    // --- Version du site ------------------------------------------------------------------------

    /// Le site en ligne correspond-il au dossier web-nomade de ce Jarvis ? Empreinte de chaque fichier
    /// (écrite par deploy.ps1) et migrations de la base.
    pub async fn check_site(&self) -> J {
        let res = match self.call("version", "", None).await {
            Ok(r) => r,
            Err(e) => {
                let out = json!({ "ok": false, "error": e, "checkedAt": js::now_ms() });
                *self.site.borrow_mut() = out.clone();
                return out;
            }
        };
        let dir = crate::paths::root().join("web-nomade");
        let local = local_manifest(&dir);
        let remote: std::collections::BTreeMap<String, String> = res["files"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), js::s(v))).collect()).unwrap_or_default();
        let (different, missing, extra) = compare_manifests(&local, &remote);
        let wanted: Vec<String> = std::fs::read_dir(dir.join("migrations")).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".sql")).collect()).unwrap_or_default();
        let applied = js::str_list(&res["migrations"]);
        let mut schema_missing: Vec<String> = wanted.into_iter().filter(|m| !applied.contains(m)).collect();
        schema_missing.sort();
        let known = !remote.is_empty();
        let up_to_date = known && different.is_empty() && missing.is_empty() && schema_missing.is_empty();
        let out = json!({
            "ok": true, "checkedAt": js::now_ms(), "jarvisVersion": env!("CARGO_PKG_VERSION"),
            "siteVersion": res["version"], "commit": res["commit"], "modifie": res["modifie"], "deployedAt": res["deployedAt"], "php": res["php"],
            "aJour": up_to_date, "connu": known, "fichiers": local.len(),
            "differents": different, "manquants": missing, "enTrop": extra, "migrationsManquantes": schema_missing,
        });
        if !up_to_date && js::gb(&self.site.borrow(), "aJour") {
            (self.log)("Nomade : le site en ligne ne correspond plus au dossier web-nomade (à déployer)".into());
        }
        *self.site.borrow_mut() = out.clone();
        out
    }

    // --- Listes des invités → mémoire de Jarvis -----------------------------------------------

    /// Listes créées par les invités sur le site : gardées dans data/nomade/amis.json et rangées dans la
    /// mémoire (un souvenir épinglé par liste, catégorie selon le lien : famille, amis…).
    async fn fetch_friends(&self) -> Result<(), String> {
        let res = self.call("friends", "", None).await?;
        let friends = js::arr(&res["friends"]);
        let o = self.o.up();
        let before: HashSet<String> = js::read_json(&self.dir.join("amis.json")).map(|a| js::str_list(&a["pins"]).into_iter().collect()).unwrap_or_default();
        let mut pins = vec![];
        let mut changed = false;
        for f in &friends {
            let (name, relation) = (js::gs(f, "name"), js::gs(f, "relation"));
            for l in js::arr(&f["lists"]) {
                let pin = format!("nomade_{}", js::gs(&l, "id"));
                let text = friend_list_text(&name, &relation, &js::gs(&l, "title"), &js::arr(&l["items"]));
                let old = o.memory.list().into_iter().find(|e| js::gs(e, "pinned") == pin).map(|e| js::gs(&e, "text"));
                if old.as_deref() != Some(text.as_str()) {
                    o.memory.upsert_pinned(&pin, &text, relation_category(&relation));
                    changed = true;
                }
                pins.push(pin);
            }
        }
        for gone in before.iter().filter(|p| !pins.contains(p)) {
            changed |= o.memory.forget_pinned(gone);
        }
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = js::write_json(&self.dir.join("amis.json"), &json!({ "friends": friends, "pins": pins, "at": js::now_ms() }));
        self.friends_dirty.set(false);
        if changed {
            o.record("nomade_listes_amis", json!({ "listes": pins.len() }));
            o.publish_memory();
            let o2 = o.clone();
            js::spawn(async move {
                let _ = o2.memory.index().await;
            });
        }
        Ok(())
    }

    // --- Partages : comptes invités du site ----------------------------------------------------

    /// Partages du site, avec les sources que Jarvis partage (seules celles-là peuvent être données).
    pub async fn guests(&self) -> J {
        if !self.configured() {
            return json!({ "ok": false, "error": "Jarvis n'est pas encore relié au site." });
        }
        match self.call("guests", "", None).await {
            Ok(r) => json!({ "ok": true, "guests": r["guests"], "sources": self.shared_labels() }),
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    fn shared_labels(&self) -> Vec<J> {
        let o = self.o.up();
        let shared = self.shared();
        sources::catalog(&o).into_iter().filter(|s| shared.contains(&s.id)).map(|s| json!({ "id": s.id, "label": s.label, "writable": !s.ops.is_empty() })).collect()
    }

    /// Création (invitation envoyée par mail) ou modification d'un partage.
    pub async fn guest_save(&self, p: &J) -> J {
        let login = js::gs(p, "login").trim().to_string();
        if !valid_login(&login) {
            return json!({ "ok": false, "error": "Identifiant de 3 caractères au moins (lettres, chiffres, . _ @ -)." });
        }
        let shared = self.shared();
        let srcs: Vec<String> = js::str_list(&p["sources"]).into_iter().filter(|s| shared.contains(s)).collect();
        let id = p["id"].as_i64().unwrap_or(0);
        let body = json!({
            "id": id, "login": login, "email": js::gs(p, "email").trim(), "label": js::slice_chars(js::gs(p, "label").trim(), 120),
            "mode": if js::gs(p, "mode") == "ecriture" { "ecriture" } else { "lecture" }, "sources": srcs, "invite": id == 0,
            "relation": if RELATIONS.contains(&js::gs(p, "relation").as_str()) { js::gs(p, "relation") } else { "ami".into() },
        });
        let res = match self.call("guest-save", "", Some(body)).await {
            Ok(r) => r,
            Err(e) => return json!({ "ok": false, "error": e }),
        };
        self.friends_dirty.set(true);
        self.poke(1000);
        let mut out = json!({ "ok": true, "guest": res["guest"] });
        if let Some(token) = res["token"].as_str() {
            js::assign(&mut out, self.send_invite(&res["guest"], token).await);
        }
        out
    }

    /// Nouveau lien d'invitation (mail perdu, mot de passe oublié) : l'ancien ne marche plus.
    pub async fn guest_invite(&self, id: i64) -> J {
        match self.call("guest-invite", "", Some(json!({ "id": id }))).await {
            Ok(r) => {
                let mut out = json!({ "ok": true, "guest": r["guest"] });
                js::assign(&mut out, self.send_invite(&r["guest"], &js::gs(&r, "token")).await);
                out
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    pub async fn guest_delete(&self, id: i64) -> J {
        match self.call("guest-delete", "", Some(json!({ "id": id }))).await {
            Ok(_) => {
                self.friends_dirty.set(true);
                self.poke(1000);
                json!({ "ok": true })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        }
    }

    /// Mail d'invitation (Gmail de Jarvis) : adresse du site, identifiant, lien pour choisir le mot de passe.
    /// Le lien est aussi rendu à l'administration (à transmettre autrement si le mail ne part pas).
    async fn send_invite(&self, guest: &J, token: &str) -> J {
        let url = js::gs(&self.config.borrow(), "url");
        let login = js::gs(guest, "login");
        let link = format!("{url}/#invite={token}&login={login}");
        let email = js::gs(guest, "email");
        let o = self.o.up();
        if email.is_empty() {
            return json!({ "link": link, "mailed": false, "mailError": "aucune adresse mail : transmets le lien toi-même" });
        }
        let owner = o.notes.author();
        let names: Vec<String> = {
            let wanted = js::str_list(&guest["sources"]);
            self.shared_labels().into_iter().filter(|s| wanted.contains(&js::gs(s, "id"))).map(|s| js::gs(&s, "label")).collect()
        };
        let hello = js::gs(guest, "label");
        let body = invite_mail(&owner, &hello, &url, &login, &link, &names, js::gs(guest, "mode") == "ecriture");
        let res = o.google.mail_envoyer(&json!({ "destinataire": email, "objet": format!("{owner} partage des informations avec toi"), "corps": body })).await;
        let (mailed, error) = match res {
            Ok(r) if js::gb(&r, "ok") => (true, String::new()),
            Ok(r) => (false, if js::truthy(&r["error"]) { js::gs(&r, "error") } else { js::gs(&r, "message") }),
            Err(e) => (false, e),
        };
        o.record("nomade_invitation", json!({ "identifiant": login, "mail": email, "envoye": mailed }));
        json!({ "link": link, "mailed": mailed, "mailError": error })
    }

    // --- Boîte de dépôt -------------------------------------------------------------------------

    async fn receive_depots(&self, o: &Rc<Orchestrator>, items: Vec<J>) {
        // Liste déposée : créée tout de suite (rien d'autre n'est déclenché) et partagée sur le site.
        let (lists, items): (Vec<J>, Vec<J>) = items.into_iter().partition(|i| js::gs(i, "kind") == "liste");
        for d in lists {
            let (title, rest) = parse_list_depot(&js::gs(&d, "text"));
            let made = o.smart_lists.create_from_items(&title, &rest);
            let state = if made.is_some() { "traite" } else { "pris" };
            let _ = self.call("inbox-ack", "", Some(json!({ "ids": [d["id"].clone()], "state": state }))).await;
            match made {
                Some(list) => {
                    let id = format!("liste:{}", js::gs(&list, "id"));
                    let mut shared = self.shared();
                    if !shared.contains(&id) {
                        shared.push(id);
                        self.config.borrow_mut()["sources"] = json!(shared);
                        self.save();
                    }
                    o.record("nomade_liste_creee", json!({ "titre": title, "elements": rest.len() }));
                    o.notify(json!({ "title": "Nomade", "source": "nomade", "card": false,
                        "text": format!("J'ai créé ta liste « {title} » depuis Nomade, avec {} élément{}.", rest.len(), if rest.len() > 1 { "s" } else { "" }) }));
                    self.poke(1500);
                }
                None => (self.log)("Nomade : liste déposée sans nom, ignorée".into()),
            }
        }
        // Demande de nouvelle liste : lue avec prudence (jamais envoyée à l'IA ni à `submit`), validée par le propriétaire.
        let (requests, items): (Vec<J>, Vec<J>) = items.into_iter().partition(|i| js::gs(i, "kind") == "nouvelle_liste");
        let mut items = items;
        let owner_login = js::s(&self.c("login"));
        for mut r in requests {
            match demande::parse_request(&js::gs(&r, "text")) {
                Some(req) => {
                    r["requester"] = demande::requester(&r, &owner_login);
                    r["demande"] = json!({ "nom": req.nom, "type": req.type_liste, "champs": req.champs, "exemples": req.exemples });
                    items.push(r);
                }
                None => {
                    let _ = self.call("inbox-ack", "", Some(json!({ "ids": [r["id"].clone()], "state": "refuse" }))).await;
                    (self.log)("Nomade : demande de liste illisible, refusée".into());
                }
            }
        }
        if items.is_empty() {
            return;
        }
        let fresh: Vec<J> = {
            let mut depots = self.depots.borrow_mut();
            let known: HashSet<i64> = depots.iter().filter_map(|d| d["id"].as_i64()).collect();
            let fresh: Vec<J> = items.into_iter().filter(|i| i["id"].as_i64().is_some_and(|id| !known.contains(&id))).map(|mut i| {
                i["receivedAt"] = json!(js::now_ms());
                i
            }).collect();
            depots.extend(fresh.iter().cloned());
            let excess = depots.len().saturating_sub(MAX_DEPOTS);
            depots.drain(..excess);
            fresh
        };
        let ids: Vec<J> = fresh.iter().map(|i| i["id"].clone()).collect();
        if !ids.is_empty() {
            self.save_depots();
            if let Err(e) = self.call("inbox-ack", "", Some(json!({ "ids": ids, "state": "pris" }))).await {
                (self.log)(format!("Nomade : accusé de réception non envoyé ({e})"));
            }
            let n = fresh.len();
            o.record("nomade_depot", json!({ "nombre": n }));
            o.notify(json!({
                "title": "Nomade", "source": "nomade",
                "text": match fresh.iter().find(|d| js::gs(d, "kind") == "nouvelle_liste") {
                    Some(d) if n == 1 => format!("{} demande une nouvelle liste : « {} ». À valider dans la card Nomade.", js::gs(&d["requester"], "nom"), js::gs(&d["demande"], "nom")),
                    _ if n == 1 => "Tu as déposé quelque chose depuis Nomade.".to_string(),
                    _ => format!("Tu as déposé {n} choses depuis Nomade."),
                },
                "rows": fresh.iter().map(|d| json!({ "title": js::slice_chars(&js::gs(d, "text"), 120), "sub": d["kind"] })).collect::<Vec<_>>(),
                "card": false,
            }));
            self.show();
        }
    }

    /// Dépôt traité depuis la card ou l'administration : note, rappel ou demande à Jarvis, ou ignoré.
    pub async fn depot(&self, id: i64, action: &str) -> J {
        let Some(d) = self.depots.borrow().iter().find(|d| d["id"].as_i64() == Some(id)).cloned() else {
            return json!({ "ok": false, "error": "Dépôt introuvable." });
        };
        let o = self.o.up();
        let text = js::gs(&d, "text");
        let request = js::gs(&d, "kind") == "nouvelle_liste";
        // Une demande de liste ne devient jamais une note, un rappel ni une demande à Jarvis.
        let ack = if request && ["ignorer", "refuser"].contains(&action) { "refuse" } else { "traite" };
        let done = match action {
            "creer_liste" if request => self.create_requested_list(&o, &d).await,
            "refuser" if request => Ok("Demande refusée.".to_string()),
            "note" | "demande" | "rappel" if request => Err("Une demande de liste se valide ou se refuse.".to_string()),
            "note" => {
                let title = text.lines().next().map(|l| js::slice_chars(l.trim(), 80)).unwrap_or_default();
                o.notes.write_new(&title, &text, "nomade").map(|_| "Note enregistrée.".to_string())
            }
            // La phrase passe par Jarvis comme une demande tapée : mêmes règles d'accord qu'au clavier.
            "demande" | "rappel" => {
                let phrase = if action == "rappel" && !js::fold_lower(&text).starts_with("rappel") { format!("rappelle-moi {text}") } else { text.clone() };
                o.submit(phrase, json!({ "source": "nomade" })).await;
                Ok("Transmis à Jarvis.".to_string())
            }
            "ignorer" => Ok("Dépôt retiré.".to_string()),
            _ => Err("Action inconnue.".to_string()),
        };
        match done {
            Err(e) => json!({ "ok": false, "error": e }),
            Ok(msg) => {
                self.depots.borrow_mut().retain(|x| x["id"].as_i64() != Some(id));
                self.save_depots();
                let _ = self.call("inbox-ack", "", Some(json!({ "ids": [id], "state": ack }))).await;
                js::spread(&json!({ "ok": true, "say": msg }), &self.state())
            }
        }
    }

    /// La card Nomade du HUD (modules/nomade) : état de la synchronisation et dépôts à traiter.
    pub fn show(&self) {
        let st = self.state();
        let shared = js::arr(&st["sources"]).iter().filter(|s| js::gb(s, "shared")).count();
        self.o.up().show_widget("nomade_view", json!({
            "title": "Nomade", "configured": st["configured"], "url": st["url"], "lastSync": st["lastSync"], "lastError": st["lastError"],
            "shared": shared, "depots": st["depots"], "conflicts": st["conflicts"]
        }));
    }

    // --- Liste demandée depuis le site -------------------------------------------------------------

    /// Étudie la demande (modèle livré ou déjà créé, sinon Claude Haiku sans outil), enregistre le modèle, crée la liste,
    /// la partage sur le site et, pour un invité, lui en accorde l'écriture (une fois la source arrivée sur le site).
    async fn create_requested_list(&self, o: &Rc<Orchestrator>, d: &J) -> Result<String, String> {
        use super::collections::schema;
        let req = demande::parse_request(&js::gs(d, "text")).ok_or("Demande illisible.")?;
        let who = if d["requester"].is_object() { d["requester"].clone() } else { demande::requester(d, &js::s(&self.c("login"))) };
        let haystack = format!("{} {}", req.nom, req.type_liste);
        let mut input: Option<J> = None;
        if req.champs.is_empty() {
            input = o.collections.find_custom_model(&haystack).or_else(|| schema::detect_preset(&haystack));
        }
        if input.is_none() {
            let login = js::gs(&who, "login");
            let mut quotas = js::read_json(&self.dir.join("ia-demandes.json")).unwrap_or(json!({}));
            let stamps: Vec<i64> = js::arr(&quotas[login.as_str()]).iter().filter_map(|t| t.as_i64()).collect();
            if demande::quota_ok(&stamps, js::now_ms()) {
                let mut kept: Vec<J> = stamps.iter().filter(|t| js::now_ms() - **t < 86_400_000).map(|t| json!(t)).collect();
                kept.push(json!(js::now_ms()));
                quotas[login.as_str()] = json!(kept);
                let _ = std::fs::create_dir_all(&self.dir);
                let _ = js::write_json(&self.dir.join("ia-demandes.json"), &quotas);
                let system = demande::study_system(&schema::FIELD_TYPES);
                match o.quick_haiku(&system, &demande::study_input(&req), 90).await {
                    Ok(answer) => {
                        let found = crate::orch::shopping::read_offers_data(std::path::Path::new(""), &answer).filter(|m| schema::normalize_schema(m).is_ok());
                        if found.is_none() {
                            (self.log)("Nomade : réponse de l'IA inutilisable pour le modèle, schéma de repli".into());
                        }
                        input = found;
                    }
                    Err(e) => (self.log)(format!("Nomade : étude du modèle impossible ({e}), schéma de repli")),
                }
                if let Some(m) = &input {
                    if let Ok(saved) = o.collections.save_custom_model(m) {
                        input = Some(saved);
                    }
                }
            } else {
                (self.log)(format!("Nomade : plafond d'études IA atteint pour {login}, schéma de repli"));
            }
        }
        let mut input = input.unwrap_or_else(|| demande::fallback_schema(&req));
        input["nom"] = json!(req.nom);
        if js::gs(&input, "modele").is_empty() {
            input["modele"] = input["id"].clone();
        }
        schema::align_with_site(&mut input);
        let origine = demande::origine(&who, d["id"].as_i64().unwrap_or(0));
        let list = o.collections.create_list_from(&input, "nomade", Some(origine))?;
        let id = js::gs(&list, "id");
        let source = format!("col:{id}");
        self.share(&source);
        if js::gs(&who, "role") != "owner" {
            let mut grants = js::read_json(&self.dir.join("grants.json")).map(|v| js::arr(&v)).unwrap_or_default();
            grants.push(json!({ "login": who["login"], "source": source, "at": js::now_ms() }));
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = js::write_json(&self.dir.join("grants.json"), &Value::Array(grants));
        }
        o.record("nomade_liste_demandee", json!({ "liste": id, "demandeur": who["login"], "role": who["role"] }));
        self.poke(1500);
        Ok(format!("Liste « {} » créée.", req.nom))
    }

    /// Écriture accordée aux invités sur leur liste demandée : après le push, quand la source existe sur le site.
    async fn apply_grants(&self) {
        let path = self.dir.join("grants.json");
        let grants = js::read_json(&path).map(|v| js::arr(&v)).unwrap_or_default();
        if grants.is_empty() {
            return;
        }
        let mut keep: Vec<J> = vec![];
        for g in grants {
            let body = json!({ "login": g["login"], "source": g["source"] });
            match self.call("guest-grant", "", Some(body)).await {
                Ok(_) => (self.log)(format!("Nomade : écriture accordée à {} sur {}", js::gs(&g, "login"), js::gs(&g, "source"))),
                Err(e) if js::now_ms() - g["at"].as_i64().unwrap_or(0) < 7 * 86_400_000 => {
                    (self.log)(format!("Nomade : accès à la liste demandée en attente ({e})"));
                    keep.push(g);
                }
                Err(_) => {}
            }
        }
        let _ = js::write_json(&path, &Value::Array(keep));
    }

    // --- Envoi : ce que Jarvis partage ----------------------------------------------------------

    async fn push(&self) -> Result<(usize, usize), String> {
        let o = self.o.up();
        let shared = self.shared();
        sources::refresh_live(&o, &shared).await;
        let catalog = sources::catalog(&o);
        let mut descriptors = vec![];
        let mut upserts: Vec<(String, String, J, String)> = vec![];
        let mut deletes: Vec<(String, String)> = vec![];
        let mut gone = vec![];
        for (pos, id) in shared.iter().enumerate() {
            // Liste supprimée dans Jarvis : effacée du site aussi.
            let Some(src) = catalog.iter().find(|s| s.id == *id) else {
                self.call("drop", "", Some(json!({ "source": id }))).await?;
                let mut sync = self.sync.borrow_mut();
                for k in ["pushed", "meta"] {
                    if let Some(m) = sync[k].as_object_mut() {
                        m.remove(id);
                    }
                }
                gone.push(id.clone());
                continue;
            };
            let desc = src.descriptor(pos);
            let dh = sources::hash(&desc);
            if self.sync.borrow()["meta"][id.as_str()].as_str() != Some(dh.as_str()) {
                descriptors.push((id.clone(), desc, dh));
            }
            let rows = sources::records(&o, id).unwrap_or_default();
            let pushed = self.sync.borrow()["pushed"][id.as_str()].as_object().cloned().unwrap_or_default();
            let (ups, dels) = diff(&pushed, &rows);
            upserts.extend(ups.into_iter().map(|(rid, d, h)| (id.clone(), rid, d, h)));
            deletes.extend(dels.into_iter().map(|rid| (id.clone(), rid)));
        }
        if !gone.is_empty() {
            let kept: Vec<String> = shared.iter().filter(|s| !gone.contains(s)).cloned().collect();
            self.config.borrow_mut()["sources"] = json!(kept);
            self.save();
            self.save_sync();
        }
        let temp: Vec<J> = js::arr(&self.sync.borrow()["tempDeletes"]);
        if descriptors.is_empty() && upserts.is_empty() && deletes.is_empty() && temp.is_empty() {
            let _ = self.call("ping", "", None).await?;
            return Ok((0, 0));
        }
        self.upload_images(&upserts).await?;
        let (sent, removed) = (upserts.len(), deletes.len());
        // Premier lot : descriptions des sources et suppressions ; puis les enregistrements par lots.
        let mut del_json: Vec<J> = deletes.iter().map(|(s, r)| json!({ "source": s, "id": r })).collect();
        del_json.extend(temp.iter().cloned());
        let first = json!({ "sources": descriptors.iter().map(|(_, d, _)| d.clone()).collect::<Vec<_>>(), "upserts": [], "deletes": del_json });
        self.call("push", "", Some(first)).await?;
        {
            let mut sync = self.sync.borrow_mut();
            for (id, _, dh) in &descriptors {
                sync["meta"][id.as_str()] = json!(dh);
            }
            for (s, r) in &deletes {
                if let Some(m) = sync["pushed"][s.as_str()].as_object_mut() {
                    m.remove(r);
                }
            }
            sync["tempDeletes"] = json!([]);
        }
        self.save_sync();
        for chunk in batches(&upserts) {
            let body = json!({ "upserts": chunk.iter().map(|(s, r, d, h)| json!({ "source": s, "id": r, "data": d, "hash": h })).collect::<Vec<_>>() });
            self.call("push", "", Some(body)).await?;
            {
                let mut sync = self.sync.borrow_mut();
                for (s, r, _, h) in chunk {
                    sync["pushed"][s.as_str()][r.as_str()] = json!(h);
                }
            }
            self.save_sync();
        }
        if removed > 0 || !temp.is_empty() {
            let _ = self.call("gc", "", Some(json!({}))).await;
        }
        Ok((sent, removed))
    }

    /// Images locales des fiches envoyées : le site dit lesquelles il n'a pas, elles partent avant les fiches.
    async fn upload_images(&self, upserts: &[(String, String, J, String)]) -> Result<(), String> {
        let mut names: Vec<String> = upserts.iter().map(|(_, _, d, _)| js::gs(d, "_img")).filter(|n| !n.is_empty()).collect();
        names.sort();
        names.dedup();
        if names.is_empty() {
            return Ok(());
        }
        let missing = js::str_list(&self.call("has", "", Some(json!({ "names": names }))).await?["missing"]);
        for name in missing {
            let Some(bytes) = sources::image_file(&name).and_then(|f| std::fs::read(f).ok()) else { continue };
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            // Échec : rien n'est envoyé ce tour-ci, la fiche et son image repartiront ensemble au suivant.
            self.call("upload", "", Some(json!({ "name": name, "data": data }))).await.map_err(|e| format!("image non envoyée ({e})"))?;
        }
        Ok(())
    }
}

/// Comparaison du site déployé avec le dossier web-nomade : au plus toutes les 10 minutes (et sur demande).
const SITE_CHECK_MS: i64 = 10 * 60 * 1000;

/// Fichiers de web-nomade qui ne partent jamais sur le site (outils, modèle, fichiers générés).
fn deploy_skipped(rel: &str) -> bool {
    rel == "deploy.ps1" || rel.ends_with("README.md") || rel == "private/config.example.php" || rel == "private/config.php" || rel == "private/version.json"
}

/// Empreinte SHA-256 de chaque fichier déployable de web-nomade (chemins en « / »), comme deploy.ps1.
pub fn local_manifest(dir: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    use sha2::{Digest, Sha256};
    fn walk(base: &std::path::Path, d: &std::path::Path, out: &mut std::collections::BTreeMap<String, String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
                continue;
            }
            let rel = p.strip_prefix(base).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            if rel.is_empty() || deploy_skipped(&rel) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&p) {
                out.insert(rel, hex::encode(Sha256::digest(&bytes)));
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// (différents, absents du site, en trop sur le site) entre deux listes d'empreintes.
pub fn compare_manifests(local: &std::collections::BTreeMap<String, String>, remote: &std::collections::BTreeMap<String, String>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let different = local.iter().filter(|(k, v)| remote.get(*k).is_some_and(|r| !r.eq_ignore_ascii_case(v))).map(|(k, _)| k.clone()).collect();
    let missing = local.keys().filter(|k| !remote.contains_key(*k)).cloned().collect();
    let extra = remote.keys().filter(|k| !local.contains_key(*k)).cloned().collect();
    (different, missing, extra)
}

/// Listes des invités : identifiants de source sur le site (« ami:<invité>-<code> »).
pub const FRIEND_PREFIX: &str = "ami:";
/// Lien avec la personne d'un partage.
pub const RELATIONS: &[&str] = &["famille", "ami", "collegue", "autre"];

/// Catégorie de mémoire d'une liste d'invité, selon le lien avec la personne.
pub fn relation_category(relation: &str) -> &'static str {
    match relation {
        "famille" => "famille",
        "collegue" => "collegues",
        "autre" => "proches",
        _ => "amis",
    }
}

/// Souvenir d'une liste d'invité : qui, quel lien, ce qu'il reste et ce qui est coché (notes comprises).
pub fn friend_list_text(name: &str, relation: &str, title: &str, items: &[J]) -> String {
    let who = match relation {
        "famille" => format!("{name} (famille)"),
        "collegue" => format!("{name} (collègue)"),
        "autre" => name.to_string(),
        _ => format!("{name} (ami)"),
    };
    let line = |d: &J| {
        let (text, note) = (js::gs(d, "text").trim().to_string(), js::gs(d, "note").trim().to_string());
        if note.is_empty() { text } else { format!("{text} ({note})") }
    };
    let data: Vec<J> = items.iter().map(|i| i["data"].clone()).filter(|d| !js::gs(d, "text").trim().is_empty()).collect();
    let open: Vec<String> = data.iter().filter(|d| !js::gb(d, "checked")).map(line).collect();
    let done: Vec<String> = data.iter().filter(|d| js::gb(d, "checked")).map(line).collect();
    let mut parts = vec![format!("Liste « {title} » de {who}, tenue par {name} sur le site Nomade.")];
    parts.push(if open.is_empty() { "Rien en attente.".into() } else { format!("En attente ({}) : {}.", open.len(), open.join(" ; ")) });
    if !done.is_empty() {
        parts.push(format!("Déjà fait / coché ({}) : {}.", done.len(), done.join(" ; ")));
    }
    js::slice_chars(&parts.join(" "), 3000)
}

/// Dépôt « liste » : première ligne = nom, puis un élément par ligne (puces et numéros retirés).
pub fn parse_list_depot(text: &str) -> (String, Vec<String>) {
    let mut lines = text.lines().map(|l| crate::re!(r"^\s*(?:[-*•·]|\d+[.)])?\s*").replace(l, "").trim().to_string()).filter(|l| !l.is_empty());
    let title = lines.next().unwrap_or_default();
    (title, lines.collect())
}

/// Texte du mail d'invitation (lignes séparées par CRLF, comme l'attend SMTP).
pub fn invite_mail(owner: &str, hello: &str, url: &str, login: &str, link: &str, names: &[String], write: bool) -> String {
    let what = if names.is_empty() { String::new() } else { format!(" : {}", names.join(", ")) };
    [
        format!("Bonjour{},", if hello.is_empty() { String::new() } else { format!(" {hello}") }),
        String::new(),
        format!("{owner} partage avec toi des informations de son assistant Jarvis{what}."),
        format!("Ton accès est {}.", if write { "en lecture et écriture : tu peux aussi ajouter et modifier" } else { "en lecture seule" }),
        String::new(),
        format!("Adresse du site : {url}"),
        format!("Ton identifiant : {login}"),
        String::new(),
        "Pour choisir ton mot de passe, ouvre ce lien (valable 7 jours) :".to_string(),
        link.to_string(),
        String::new(),
        format!("Ensuite, connecte-toi sur {url} avec ton identifiant et ton mot de passe ; la connexion reste mémorisée."),
        "Sur un téléphone, « Ajouter à l'écran d'accueil » l'installe comme une application.".to_string(),
    ]
    .join("\r\n")
}

/// Lots d'envoi : BATCH enregistrements au plus, et BATCH_BYTES environ (un enregistrement trop gros part seul).
fn batches<T: AsRef<[(String, String, J, String)]>>(all: &T) -> Vec<&[(String, String, J, String)]> {
    let all = all.as_ref();
    let mut out = vec![];
    let (mut start, mut bytes) = (0, 0);
    for (i, (_, _, d, _)) in all.iter().enumerate() {
        let size = d.to_string().len() + 200;
        if i > start && (i - start >= BATCH || bytes + size > BATCH_BYTES) {
            out.push(&all[start..i]);
            start = i;
            bytes = 0;
        }
        bytes += size;
    }
    if start < all.len() {
        out.push(&all[start..]);
    }
    out
}

/// « synchronise le site nomade », « montre nomade », « qu'est-ce que j'ai déposé ? »
pub(crate) fn intents(o: &OrchRef, t: &str) -> Option<Intent> {
    if crate::re!(r"^(?:synchronise|synchroniser|resynchronise|mets a jour|actualise) (?:le |la |mon |ma |l )?(?:site |card |carte |carde |module |appli |application )?(?:nomade?|telephone)$").is_match(t) {
        return Some(it(o, "nomade_sync", "Nomade", "synchronisation du site", |o| async move {
            if !js::gb(&o.nomade.state(), "configured") {
                return json!({ "say": "Le site Nomade n'est pas encore relié : va dans l'administration, section Nomade.", "display": "Nomade non relié" });
            }
            let r = o.nomade.sync_now().await;
            if !js::gb(&r, "ok") {
                return json!({ "say": format!("La synchronisation a échoué : {}.", js::gs(&r, "error")), "display": "Nomade : échec" });
            }
            let s = &r["result"];
            let (sent, got) = (js::gn_or(s, "envoyes", 0.0) as i64, js::gn_or(s, "recus", 0.0) as i64);
            let say = match (sent, got) {
                (0, 0) => "Le site Nomade est déjà à jour.".to_string(),
                _ => format!("Site Nomade à jour : {sent} élément{} envoyé{}, {got} modification{} reprise{}.", if sent > 1 { "s" } else { "" }, if sent > 1 { "s" } else { "" }, if got > 1 { "s" } else { "" }, if got > 1 { "s" } else { "" }),
            };
            o.nomade.show();
            json!({ "say": say, "display": "Nomade synchronisé" })
        }));
    }
    if crate::re!(r"^(?:(?:montre|montrez|montrer|affiche|affichez|afficher|ouvre|ouvrez|ouvrir|lance|lancez|lancer|demarre|demarrer|active|activer|fais voir)(?: moi)? (?:le |la |mon |ma )?(?:(?:site|card|cards|carte|cartes|carde|cardes|module|modules|widget|widgets|panneau|fenetre|fiche|appli|application) (?:de |du |d )?)?nomade?(?: \d{1,3})?|(?:la |le )?(?:site|card|carte|carde|module) (?:de )?nomade?(?: \d{1,3})?|mes depots(?: nomade?)?|depots nomade?|(?:qu est ce que j ai|qu ai je) depose(?: dehors| sur nomade?| sur le site| depuis le telephone| sur le telephone)?)(?: s il te plait| s il vous plait| stp| svp| merci)?$").is_match(t) {
        return Some(it(o, "nomade_show", "Nomade", "dépôts et état du site", |o| async move {
            let n = js::arr(&o.nomade.state()["depots"]).len();
            o.nomade.show();
            let say = match n {
                0 => "Rien n'a été déposé depuis le site Nomade.".to_string(),
                1 => "Un dépôt t'attend depuis le site Nomade.".to_string(),
                n => format!("{n} dépôts t'attendent depuis le site Nomade."),
            };
            json!({ "say": say, "display": "Nomade" })
        }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_envoie_seulement_ce_qui_change() {
        let a = json!({ "_t": "Abbey Road" });
        let b = json!({ "_t": "Revolver" });
        let mut pushed = Map::new();
        pushed.insert("1".into(), json!(sources::hash(&a)));
        pushed.insert("3".into(), json!("ancienne"));
        let (ups, dels) = diff(&pushed, &[("1".into(), a.clone()), ("2".into(), b.clone())]);
        assert_eq!(ups.iter().map(|u| u.0.as_str()).collect::<Vec<_>>(), vec!["2"]);
        assert_eq!(dels, vec!["3".to_string()]);
        let (ups, _) = diff(&pushed, &[("1".into(), json!({ "_t": "Abbey Road (remaster)" }))]);
        assert_eq!(ups.len(), 1, "une fiche modifiée repart");
    }

    #[test]
    fn conflit_seulement_si_valeurs_differentes() {
        let local = json!({ "artiste": "The Beatles", "annee": 1969, "_t": "Abbey Road" });
        let f = conflicting_fields(&local, &json!({ "artiste": "Beatles", "annee": 1969, "_t": "x" }));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["champ"], json!("artiste"));
        assert!(conflicting_fields(&local, &json!({ "nouveau": "valeur" })).is_empty(), "champ vide côté Jarvis : pas de conflit");
    }

    #[test]
    fn ecarts_identiques_regroupes() {
        let dup = |at: i64, id: &str| json!({ "source": "col:lego", "id": id, "titre": "Code-barres 1", "erreur": "déjà dans la liste : non ajouté", "at": at });
        let refus = |id: &str| json!({ "source": "col:noel", "id": id, "erreur": "lien", "site": { "_t": "Enceinte" }, "at": 5 });
        let out = merge_conflicts(vec![dup(9, "w-a"), refus("w-x"), dup(8, "w-b"), refus("w-y"), dup(7, "w-c")]);
        assert_eq!(out.len(), 3, "les refus restent séparés (chacun se réessaie)");
        assert_eq!((out[0]["at"].clone(), out[0]["fois"].clone()), (json!(9), json!(3)), "le plus récent garde le compte");
        let again = merge_conflicts(vec![dup(10, "w-d"), out[0].clone()]);
        assert_eq!(again[0]["fois"], json!(4));
    }

    #[test]
    fn secrets_jamais_en_clair() {
        let cfg = json!({ "url": "https://x.fr", "login": "moi", "password": "motdepasse-123", "key": "cle-appareil", "sources": [] });
        let stored = Nomade::stored_form(&cfg, |s| Some(format!("chiffre:{}", s.len())));
        let text = stored.to_string();
        assert!(!text.contains("motdepasse-123") && !text.contains("cle-appareil"));
        assert_eq!(stored["passwordEnc"], json!("chiffre:14"));
        let none = Nomade::stored_form(&cfg, |_| None);
        assert!(!none.to_string().contains("motdepasse-123"), "sans chiffrement, le secret n'est pas écrit");
        assert!(none.get("password").is_none() && none.get("passwordEnc").is_none());
    }

    #[test]
    fn adresse_et_identifiant() {
        assert_eq!(clean_url(" https://jarvis.exemple.fr/ ").unwrap(), "https://jarvis.exemple.fr");
        assert!(clean_url("http://jarvis.exemple.fr").is_err());
        assert!(clean_url("https://x.fr/?a=1").is_err());
        assert!(valid_login("johann.l@maison") && !valid_login("ab") && !valid_login("a b c"));
    }

    #[test]
    fn lots_limites_en_nombre_et_en_taille() {
        let small: Vec<_> = (0..600).map(|i| ("s".to_string(), i.to_string(), json!({ "_t": "x" }), String::new())).collect();
        assert_eq!(batches(&small).iter().map(|b| b.len()).collect::<Vec<_>>(), vec![250, 250, 100]);
        let big: Vec<_> = (0..5).map(|i| ("s".to_string(), i.to_string(), json!({ "body": "a".repeat(700_000) }), String::new())).collect();
        assert!(batches(&big).iter().all(|b| b.len() <= 2), "environ 1,5 Mo par lot");
        assert_eq!(batches(&big).iter().map(|b| b.len()).sum::<usize>(), 5);
    }

    #[test]
    fn image_nommee_par_son_contenu() {
        let dir = std::env::temp_dir().join(format!("nomade-img-{}", js::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("e1.jpg");
        std::fs::write(&f, b"pochette").unwrap();
        let name = sources::image_name(&f).unwrap();
        assert!(name.ends_with(".jpg") && name.len() == 68, "{name}");
        assert_eq!(sources::image_file(&name).as_deref(), Some(f.as_path()));
        std::fs::write(dir.join("e2.jpg"), b"pochette").unwrap();
        assert_eq!(sources::image_name(&dir.join("e2.jpg")).unwrap(), name, "même contenu, même nom");
        std::fs::write(dir.join("e3.exe"), b"x").unwrap();
        assert!(sources::image_name(&dir.join("e3.exe")).is_none());
        std::fs::write(dir.join("e4.png"), vec![0u8; sources::MAX_IMAGE_BYTES as usize + 1]).unwrap();
        assert!(sources::image_name(&dir.join("e4.png")).is_none(), "trop lourde");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Le site répond en HTTPS au client de Jarvis (certificat accepté). Réseau : à lancer à la main,
    /// `NOMADE_URL=https://… cargo test -p jarvis-core site_nomade -- --ignored`.
    #[test]
    #[ignore]
    fn site_nomade_joignable() {
        let Some(url) = js::env("NOMADE_URL") else { return };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let res = rt.block_on(crate::net::http().get(format!("{}/api/?r=ping", url.trim_end_matches('/'))).send()).expect("HTTPS refusé");
        assert_eq!(res.status().as_u16(), 401, "sans clé, le site doit répondre 401");
    }

    #[test]
    fn mail_d_invitation() {
        let m = invite_mail("Johann", "Marie", "https://site.exemple", "marie", "https://site.exemple/#invite=abc&login=marie", &["Lego".into(), "Courses".into()], false);
        assert!(m.starts_with("Bonjour Marie,\r\n"));
        assert!(m.contains("Ton identifiant : marie") && m.contains("#invite=abc&login=marie") && m.contains("lecture seule") && m.contains(": Lego, Courses."));
        assert!(!m.lines().any(|l| l.starts_with('.')), "pas de ligne commençant par un point (SMTP)");
    }

    #[test]
    fn liste_d_invite_en_souvenir() {
        let items = vec![
            json!({ "data": { "text": "Vélo", "note": "rouge, taille M", "checked": false } }),
            json!({ "data": { "text": "Livre", "checked": true } }),
            json!({ "data": { "text": "  " } }),
        ];
        let t = friend_list_text("Marie", "famille", "Cadeaux de Noël", &items);
        assert!(t.starts_with("Liste « Cadeaux de Noël » de Marie (famille)"), "{t}");
        assert!(t.contains("En attente (1) : Vélo (rouge, taille M).") && t.contains("Déjà fait / coché (1) : Livre."), "{t}");
        assert_eq!(relation_category("famille"), "famille");
        assert_eq!(relation_category("inconnu"), "amis");
        assert!(friend_list_text("Paul", "ami", "Vide", &[]).contains("Rien en attente."));
    }

    #[test]
    fn fiche_du_site_gardee_meme_avec_un_lien_trop_long() {
        // Adresse de boutique de plus de 500 caractères (pistage) : raccourcie sans la requête, jamais perdue.
        let long = format!("https://www.amazon.fr/Marshall-Acton/dp/B0B1QPMNYR/ref=sr_1_1?{}", "dib=abcdefghij".repeat(60));
        assert!(long.chars().count() > 500);
        let short = crate::orch::shopping::safe_url(&json!(long)).unwrap();
        assert_eq!(short, "https://www.amazon.fr/Marshall-Acton/dp/B0B1QPMNYR/ref=sr_1_1");
        // Repli : sans les champs lien et image de la liste, la fiche reste ajoutable.
        let list = json!({ "champs": [{ "cle": "nom", "type": "texte" }, { "cle": "lien", "type": "lien" }, { "cle": "photo", "type": "image" }] });
        let values = json!({ "nom": "Enceinte", "lien": "pas une adresse", "photo": "x" });
        assert_eq!(without_links(Some(&list), &values), json!({ "nom": "Enceinte" }));
        assert_eq!(without_links(None, &values), values);
    }

    #[test]
    fn liste_deposee() {
        let (t, items) = parse_list_depot("Courses samedi\n- lait\n\n2) œufs\n• pain complet ");
        assert_eq!(t, "Courses samedi");
        assert_eq!(items, vec!["lait", "œufs", "pain complet"]);
        assert_eq!(parse_list_depot("").0, "");
    }

    #[test]
    fn code_barres_nettoye() {
        use super::super::collections::service::clean_barcode;
        assert_eq!(clean_barcode(" 5702-0175 83556 ").as_deref(), Some("5702017583556"));
        assert!(clean_barcode("12345").is_none() && clean_barcode("57020A7583556").is_none());
    }

    #[test]
    fn disque_par_code_barres() {
        use super::super::collections::enrich::parse_mb_barcode;
        let v = json!({ "releases": [
            { "title": "Abbey Road", "date": "1969-09-26", "artist-credit": [{ "name": "The Beatles" }], "release-group": { "id": "9162580e-5df4-32de-80cc-f45a8d8a9b1d" } },
            { "title": "Abbey Road", "date": "2019", "release-group": { "id": "9162580e-5df4-32de-80cc-f45a8d8a9b1d" } },
            { "title": "x", "release-group": { "id": "pas-un-id" } }
        ] });
        let c = parse_mb_barcode(&v);
        assert_eq!(c.len(), 1, "même album une seule fois, identifiant invalide écarté");
        assert_eq!((c[0]["titre"].as_str(), c[0]["artiste"].as_str(), c[0]["annee"].as_i64()), (Some("Abbey Road"), Some("The Beatles"), Some(1969)));
    }

    #[test]
    fn version_du_site_par_empreintes() {
        let dir = std::env::temp_dir().join(format!("nomade-manifest-{}", js::now_ms()));
        std::fs::create_dir_all(dir.join("api")).unwrap();
        std::fs::create_dir_all(dir.join("private")).unwrap();
        std::fs::write(dir.join("app.js"), b"a").unwrap();
        std::fs::write(dir.join("api").join("index.php"), b"b").unwrap();
        for skip in ["deploy.ps1", "README.md", "private/config.php", "private/version.json", "private/config.example.php"] {
            std::fs::write(dir.join(skip), b"x").unwrap();
        }
        let local = local_manifest(&dir);
        assert_eq!(local.keys().cloned().collect::<Vec<_>>(), vec!["api/index.php".to_string(), "app.js".to_string()], "outils et fichiers générés exclus");
        assert_eq!(local["app.js"], "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb", "même empreinte que Get-FileHash (en minuscules)");
        let mut remote = local.clone();
        remote.insert("app.js".into(), local["app.js"].to_uppercase());
        assert_eq!(compare_manifests(&local, &remote), (vec![], vec![], vec![]), "majuscules ou minuscules : même empreinte");
        remote.insert("app.js".into(), "0".repeat(64));
        remote.remove("api/index.php");
        remote.insert("vieux.js".into(), "1".repeat(64));
        assert_eq!(compare_manifests(&local, &remote), (vec!["app.js".to_string()], vec!["api/index.php".to_string()], vec!["vieux.js".to_string()]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn phrases_reconnues() {
        let o: OrchRef = Weak::new();
        for t in ["synchronise le site nomade", "synchronise mon telephone", "mets a jour nomade", "montre nomade", "mes depots", "qu est ce que j ai depose dehors"] {
            assert!(intents(&o, t).is_some(), "{t}");
        }
        assert!(intents(&o, "synchronise").is_none());
    }

    #[test]
    fn valeurs_sans_champs_reserves() {
        let (v, s) = plain_values(&json!({ "_t": "Titre", "_statut": "possede", "titre": "Abbey Road" }));
        assert_eq!(v, json!({ "titre": "Abbey Road" }));
        assert_eq!(s, "possede");
    }
}
