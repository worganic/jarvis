// Spotify intégré (orchestrator/spotify.js) : recherche par l'API officielle (comptes Premium), sinon
// par le catalogue public (DuckDuckGo + page « embed » de Spotify, Wikidata en secours), lecture dans
// l'application Spotify du PC, démarrée discrètement et remise en arrière-plan. Les identifiants
// d'application sont chiffrés par DPAPI ; le secret n'est jamais renvoyé ni journalisé.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

use crate::js::{self, J};
use crate::secret_box::SecretBox;

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const API: &str = "https://api.spotify.com/v1";
const TIMEOUT: Duration = Duration::from_millis(6000);
const LAUNCH_WAIT_MS: i64 = 9000;
const SETTLE_MS: u64 = 2500;
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";
const STORE_APP_ID: &str = "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify";

fn norm(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

/// Minimise les fenêtres de Spotify : Windows rend le focus à la fenêtre précédente.
pub fn minimize_spotify() {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible, ShowWindow, SW_MINIMIZE};
    unsafe extern "system" fn cb(h: HWND, _l: LPARAM) -> windows::core::BOOL {
        if !IsWindowVisible(h).as_bool() {
            return true.into();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if jarvis_native::window::process_info(pid).0.eq_ignore_ascii_case("spotify") && !jarvis_native::window::window_text(h).is_empty() {
            let _ = ShowWindow(h, SW_MINIMIZE);
        }
        true.into()
    }
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(0));
    }
}

pub struct Spotify {
    secret_box: Option<Rc<SecretBox>>,
    config_file: PathBuf,
    log: Rc<dyn Fn(String)>,
    market: String,
    pub token: RefCell<Option<(String, i64)>>,
    pub last_error: RefCell<Option<String>>,
    pub premium_required: Cell<bool>,
    page_cache: RefCell<Vec<(String, J)>>,
    pub config: RefCell<J>,
}

impl Spotify {
    pub fn new(config_file: PathBuf, secret_box: Option<Rc<SecretBox>>, log: Rc<dyn Fn(String)>) -> Self {
        let s = Self {
            secret_box,
            config_file,
            log,
            market: "FR".into(),
            token: RefCell::new(None),
            last_error: RefCell::new(None),
            premium_required: Cell::new(false),
            page_cache: RefCell::new(vec![]),
            config: RefCell::new(json!({})),
        };
        *s.config.borrow_mut() = s.load_config();
        s
    }

    fn load_config(&self) -> J {
        let base = json!({ "clientId": "", "clientSecret": "", "mode": "direct" });
        let Some(mut raw) = js::read_json(&self.config_file) else { return base };
        if js::truthy(&raw["clientSecretEnc"]) {
            if let Some(b) = &self.secret_box {
                raw["clientSecret"] = json!(b.decrypt(&js::gs(&raw, "clientSecretEnc")).unwrap_or_default());
            }
        }
        if let Some(o) = raw.as_object_mut() {
            o.remove("clientSecretEnc");
        }
        js::spread(&base, &raw)
    }

    fn c(&self, k: &str) -> String {
        js::gs(&self.config.borrow(), k)
    }

    pub fn save_config(&self, patch: J) -> J {
        let merged = js::spread(&self.config.borrow(), &patch);
        *self.config.borrow_mut() = merged.clone();
        *self.token.borrow_mut() = None;
        *self.last_error.borrow_mut() = None;
        self.premium_required.set(js::gs(&merged, "mode") == "direct");
        let mut stored = merged;
        if let (Some(b), true) = (&self.secret_box, js::truthy(&stored["clientSecret"])) {
            if let Ok(enc) = b.encrypt(&js::gs(&stored, "clientSecret")) {
                stored["clientSecretEnc"] = json!(enc);
                stored.as_object_mut().unwrap().remove("clientSecret");
            }
        }
        let _ = js::write_json(&self.config_file, &stored);
        json!({ "ok": true })
    }

    pub fn activate_direct_mode(&self) -> J {
        self.premium_required.set(true);
        self.save_config(json!({ "mode": "direct" }));
        (self.log)("[spotify] Solution A activée : contrôle direct de l’application Spotify Windows".into());
        json!({ "ok": true, "mode": "direct", "message": "Solution A activée : contrôle direct de l’application Spotify sous Windows sans clé API." })
    }

    pub fn forget(&self) -> J {
        *self.config.borrow_mut() = json!({ "clientId": "", "clientSecret": "", "mode": "direct" });
        *self.token.borrow_mut() = None;
        *self.last_error.borrow_mut() = None;
        let _ = std::fs::remove_file(&self.config_file);
        json!({ "ok": true })
    }

    pub fn is_configured(&self) -> bool {
        !self.c("clientId").is_empty() && !self.c("clientSecret").is_empty()
    }

    pub fn state(&self) -> J {
        let direct = self.c("mode") == "direct" || self.premium_required.get() || !self.is_configured();
        let id = self.c("clientId");
        json!({
            "configured": self.is_configured(),
            "mode": if direct { "direct" } else { "api" },
            "modeLabel": if direct { "Solution A : Contrôle direct Windows (sans clé)" } else { "Mode API Web (Premium)" },
            "clientId": if id.is_empty() { String::new() } else { format!("{}…", js::slice_chars(&id, 6)) },
            "hasSecret": !self.c("clientSecret").is_empty(),
            "encrypted": self.secret_box.is_some(),
            "lastError": *self.last_error.borrow(),
            "premiumRequired": self.premium_required.get(),
            "directModeActive": direct
        })
    }

    fn explain403(&self, detail: &str) -> String {
        self.premium_required.set(true);
        let mut parts = vec!["Spotify refuse l’accès (403 Forbidden).".to_string()];
        let d = detail.trim();
        if !d.is_empty() && !d.eq_ignore_ascii_case("forbidden") {
            parts.push(format!("Détail Spotify : « {d} »."));
        }
        parts.extend([
            "Causes principales depuis 2026 :".to_string(),
            "1. Compte Premium obligatoire : le compte Spotify propriétaire de l’application sur developer.spotify.com doit impérativement avoir un abonnement Spotify Premium actif pour utiliser l’API Web.".to_string(),
            "2. Utilisateur non autorisé (Allowlist) : dans le Spotify Developer Dashboard → ton application → Settings → « User Management », tu dois ajouter le nom et l’adresse email de ton compte Spotify.".to_string(),
            "3. API Web non sélectionnée : lors de la création de l’application, la case « Web API » doit être cochée.".to_string(),
            "Solution alternative sans clé : si tu as un compte gratuit ou sans API, Jarvis peut piloter directement l’application Spotify de ton PC (recherche automatique, Play, Pause, Suivant) sans aucune clé !".to_string(),
        ]);
        parts.join(" ")
    }

    fn error_detail(body: &str) -> String {
        match serde_json::from_str::<J>(body) {
            Ok(b) => {
                let e = &b["error"];
                if e.is_object() {
                    if js::truthy(&e["message"]) {
                        return js::gs(e, "message");
                    }
                }
                if js::truthy(&b["error_description"]) {
                    return js::gs(&b, "error_description");
                }
                if e.is_string() {
                    return js::s(e);
                }
                js::gs(&b, "message")
            }
            Err(_) => body.to_string(),
        }
    }

    /// Jeton « identifiants d'application » : aucun compte utilisateur, seulement la recherche.
    async fn access_token(&self) -> Result<String, String> {
        if let Some((v, exp)) = self.token.borrow().as_ref() {
            if *exp > js::now_ms() + 30000 {
                return Ok(v.clone());
            }
        }
        if !self.is_configured() {
            return Err("Spotify n’est pas configuré".into());
        }
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.c("clientId"), self.c("clientSecret")));
        let res = crate::net::http()
            .post(TOKEN_URL)
            .header("Authorization", format!("Basic {basic}"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .timeout(TIMEOUT)
            .send()
            .await;
        let out = match res {
            Err(e) if e.is_timeout() => Err("Spotify ne répond pas".to_string()),
            Err(e) => Err(e.to_string()),
            Ok(resp) => {
                let status = resp.status().as_u16();
                let text = resp.text().await.unwrap_or_default();
                if !(200..300).contains(&status) {
                    let d = Self::error_detail(&text);
                    Err(match status {
                        400 | 401 => format!("identifiants refusés par Spotify (vérifie le client ID et le secret){}", if d.is_empty() { String::new() } else { format!(" : {d}") }),
                        403 => self.explain403(&d),
                        _ => format!("Spotify a répondu {status}{}", if d.is_empty() { String::new() } else { format!(" : {}", js::slice_chars(&d, 120)) }),
                    })
                } else {
                    let data: J = serde_json::from_str(&text).unwrap_or(json!({}));
                    let v = js::gs(&data, "access_token");
                    let exp = js::now_ms() + (js::gn_or(&data, "expires_in", 3600.0) * 1000.0) as i64;
                    *self.token.borrow_mut() = Some((v.clone(), exp));
                    *self.last_error.borrow_mut() = None;
                    Ok(v)
                }
            }
        };
        if let Err(e) = &out {
            *self.last_error.borrow_mut() = Some(e.clone());
        }
        out
    }

    async fn api(&self, endpoint: &str) -> Result<J, String> {
        let token = self.access_token().await?;
        let res = crate::net::http().get(format!("{API}{endpoint}")).bearer_auth(token).timeout(TIMEOUT).send().await;
        let out = match res {
            Err(e) if e.is_timeout() => Err("Spotify ne répond pas".to_string()),
            Err(e) => Err(e.to_string()),
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status == 401 {
                    *self.token.borrow_mut() = None;
                    Err("jeton Spotify expiré".to_string())
                } else if !(200..300).contains(&status) {
                    let d = Self::error_detail(&resp.text().await.unwrap_or_default());
                    Err(match status {
                        403 => self.explain403(&d),
                        429 => "Spotify limite le nombre de requêtes : réessaie dans un instant.".into(),
                        _ => format!("Spotify a répondu {status}{}", if d.is_empty() { String::new() } else { format!(" : {}", js::slice_chars(&d, 120)) }),
                    })
                } else {
                    resp.json::<J>().await.map_err(|e| e.to_string())
                }
            }
        };
        if let Err(e) = &out {
            *self.last_error.borrow_mut() = Some(e.clone());
        }
        out
    }

    async fn search(&self, query: &str, types: &str, limit: u32) -> Result<J, String> {
        self.api(&format!("/search?q={}&type={types}&limit={limit}&market={}", urlencoding::encode(query), self.market)).await
    }

    /// Toujours résolu vers un TITRE (seule adresse qui lance la lecture).
    async fn resolve(&self, raw: &str) -> Result<J, String> {
        let query = raw.trim();
        if query.is_empty() {
            return Err("rien à chercher".into());
        }
        let as_playlist = crate::re!(r"(?i)^(?:la\s+|ma\s+|une\s+)?playliste?\s+(.+)$").captures(query).map(|c| c[1].trim().to_string());
        let as_album = crate::re!(r"(?i)^(?:l['’]\s*)?album\s+(.+)$").captures(query).map(|c| c[1].trim().to_string());
        let as_artist = crate::re!(r"(?i)^(?:du|de la|des|de l['’]\s*)\s*(.+)$").captures(query).map(|c| c[1].trim().to_string());
        if let Some(p) = &as_playlist {
            if let Some(f) = self.first_playlist_track(p).await? {
                return Ok(f);
            }
        }
        if let Some(a) = &as_album {
            if let Some(f) = self.first_album_track(a).await? {
                return Ok(f);
            }
        }
        let terms = as_playlist.or(as_album).or(as_artist).unwrap_or_else(|| query.to_string());
        let data = self.search(&terms, "track,artist,playlist,album", 8).await?;
        let wanted = norm(&terms);
        if let Some(artist) = js::arr(&data["artists"]["items"]).into_iter().find(|a| js::truthy(a) && norm(&js::gs(a, "name")) == wanted) {
            let top = self.api(&format!("/artists/{}/top-tracks?market={}", js::gs(&artist, "id"), self.market)).await?;
            if let Some(t) = js::arr(&top["tracks"]).first() {
                return Ok(Self::track_info(t, Some(js::gs(&artist, "name")), "artist"));
            }
        }
        if let Some(t) = js::arr(&data["tracks"]["items"]).first() {
            return Ok(Self::track_info(t, None, "track"));
        }
        if let Some(pl) = js::arr(&data["playlists"]["items"]).into_iter().find(js::truthy) {
            if let Some(f) = self.playlist_track(&pl).await? {
                return Ok(f);
            }
        }
        Err(format!("rien trouvé pour « {query} »"))
    }

    fn track_info(track: &J, origin: Option<String>, kind: &str) -> J {
        let artists: Vec<String> = js::arr(&track["artists"]).iter().map(|a| js::gs(a, "name")).filter(|n| !n.is_empty()).collect();
        let image = js::arr(&track["album"]["images"]).into_iter().next();
        json!({
            "uri": track["uri"], "kind": kind, "origin": origin, "title": track["name"], "artist": artists.join(", "),
            "album": js::gs(&track["album"], "name"), "duration": (js::num(&track["duration_ms"]).unwrap_or(0.0) / 1000.0).round() as i64,
            "thumbnail": image.map(|i| i["url"].clone()).unwrap_or(Value::Null)
        })
    }

    async fn first_playlist_track(&self, name: &str) -> Result<Option<J>, String> {
        let data = self.search(name, "playlist", 5).await?;
        match js::arr(&data["playlists"]["items"]).into_iter().find(js::truthy) {
            Some(p) => self.playlist_track(&p).await,
            None => Ok(None),
        }
    }

    async fn playlist_track(&self, playlist: &J) -> Result<Option<J>, String> {
        let data = self.api(&format!("/playlists/{}/tracks?limit=30&market={}", js::gs(playlist, "id"), self.market)).await?;
        let items: Vec<J> = js::arr(&data["items"]).into_iter().map(|i| i["track"].clone()).filter(|t| js::truthy(&t["uri"])).collect();
        if items.is_empty() {
            return Ok(None);
        }
        let t = &items[js::rand_below(items.len())];
        Ok(Some(Self::track_info(t, Some(js::gs(playlist, "name")), "playlist")))
    }

    async fn first_album_track(&self, name: &str) -> Result<Option<J>, String> {
        let data = self.search(name, "album", 5).await?;
        let Some(album) = js::arr(&data["albums"]["items"]).into_iter().find(js::truthy) else { return Ok(None) };
        let tracks = self.api(&format!("/albums/{}/tracks?limit=1&market={}", js::gs(&album, "id"), self.market)).await?;
        let Some(t) = js::arr(&tracks["items"]).into_iter().next() else { return Ok(None) };
        let mut t = t;
        t["album"] = album.clone();
        Ok(Some(Self::track_info(&t, Some(js::gs(&album, "name")), "album")))
    }

    // --- Sans clé ni Premium : catalogue public -----------------------------------------------------

    async fn fetch_text(&self, url: &str, form: Option<&[(&str, &str)]>) -> String {
        let client = crate::net::http();
        let req = match form {
            Some(f) => client.post(url).form(f),
            None => client.get(url),
        };
        match req.header("User-Agent", BROWSER_UA).header("Accept-Language", "fr-FR,fr;q=0.9").timeout(TIMEOUT).send().await {
            Ok(r) if r.status().as_u16() == 200 => r.text().await.unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn clean_query(query: &str) -> String {
        crate::re!(r"(?i)^(?:de l['’]\s*|(?:un peu de|du|de la|des|de)\s+)").replace(query.trim(), "").trim().to_string()
    }

    fn public_query(raw: &str) -> (Option<&'static str>, String) {
        let q = raw.trim();
        if let Some(c) = crate::re!(r"(?i)^(?:la\s+|ma\s+|une\s+)?playliste?\s+(?:de\s+|d['’]\s*)?(.+)$").captures(q) {
            return (Some("playlist"), c[1].trim().to_string());
        }
        if let Some(c) = crate::re!(r"(?i)^(?:l['’]\s*|un\s+)?album\s+(?:de\s+|d['’]\s*)?(.+)$").captures(q) {
            return (Some("album"), c[1].trim().to_string());
        }
        (None, Self::clean_query(q))
    }

    async fn find_public_page(&self, typ: Option<&str>, terms: &str) -> Option<J> {
        let key = format!("{}|{}", typ.unwrap_or("*"), norm(terms));
        if let Some((_, p)) = self.page_cache.borrow().iter().find(|(k, _)| *k == key) {
            return Some(p.clone());
        }
        let scope = match typ {
            Some(t) => format!("open.spotify.com/{t}"),
            None => "open.spotify.com".into(),
        };
        let q = format!("site:{scope} {terms}");
        let html = self.fetch_text("https://html.duckduckgo.com/html/", Some(&[("q", &q), ("kl", "fr-fr")])).await;
        if html.is_empty() {
            return None;
        }
        const STOP: &[&str] = &["et", "de", "des", "du", "la", "le", "les", "un", "une", "ma", "mon", "mes", "the", "and", "of"];
        let wanted: Vec<String> = norm(terms).split(' ').filter(|w| w.chars().count() > 1 && !STOP.contains(w)).map(String::from).collect();
        let mut pages: Vec<(J, usize, usize)> = vec![];
        for m in crate::re!(r#"(?s)<a[^>]+class="result__a"[^>]+href="([^"]+)"[^>]*>(.*?)</a>"#).captures_iter(&html) {
            if pages.len() >= 10 {
                break;
            }
            let mut href = m[1].replace("&amp;", "&");
            if let Some(t) = crate::re!(r"[?&]uddg=([^&]+)").captures(&href) {
                href = urlencoding::decode(&t[1]).map(|s| s.into_owned()).unwrap_or(href.clone());
            }
            let Some(page) = crate::re!(r"open\.spotify\.com/(?:intl-[a-z]{2}/)?(track|album|playlist|artist)/([A-Za-z0-9]{22})").captures(&href) else { continue };
            if typ.map(|t| t != &page[1]).unwrap_or(false) {
                continue;
            }
            let title = crate::re!(r"<[^>]+>").replace_all(&m[2], "").replace("&amp;", "&").replace("&#x27;", "'").replace("&#39;", "'").replace("&quot;", "\"").trim().to_string();
            let words: Vec<String> = norm(&title).split(' ').map(String::from).collect();
            let score = wanted.iter().filter(|w| words.contains(w)).count();
            let rank = pages.len();
            pages.push((json!({ "type": &page[1], "id": &page[2], "title": title, "score": score, "rank": rank }), score, rank));
        }
        pages.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
        let best = pages.into_iter().next().map(|p| p.0);
        if let Some(b) = &best {
            let mut cache = self.page_cache.borrow_mut();
            if cache.len() > 100 {
                cache.remove(0);
            }
            cache.push((key, b.clone()));
        }
        best
    }

    async fn find_wikidata_page(&self, typ: Option<&str>, terms: &str) -> Option<J> {
        let prop = if typ == Some("album") { "P2205" } else { "P1902" };
        let api = "https://www.wikidata.org/w/api.php?format=json";
        let get = |url: String| async move {
            let r = crate::net::http().get(url).header("User-Agent", "Jarvis/4 (assistant vocal local)").timeout(TIMEOUT).send().await.ok()?;
            if !r.status().is_success() {
                return None;
            }
            r.json::<J>().await.ok()
        };
        let found = get(format!("{api}&action=wbsearchentities&language=fr&uselang=fr&type=item&limit=5&search={}", urlencoding::encode(terms))).await?;
        let ids: Vec<String> = js::arr(&found["search"]).iter().map(|s| js::gs(s, "id")).filter(|s| !s.is_empty()).collect();
        if ids.is_empty() {
            return None;
        }
        let data = get(format!("{api}&action=wbgetentities&props=claims|labels&languages=fr|en&ids={}", ids.join("|"))).await;
        let wanted_s = crate::re!(r"\b(?:et|and|the|les?|la|de|du|des)\b").replace_all(&norm(terms), " ").into_owned();
        let wanted: Vec<&str> = wanted_s.split(' ').filter(|w| !w.is_empty()).collect();
        for id in &ids {
            let entity = data.as_ref().map(|d| d["entities"][id].clone()).unwrap_or(Value::Null);
            let sid = js::gs(&entity["claims"][prop][0]["mainsnak"]["datavalue"], "value");
            if !crate::re!(r"^[A-Za-z0-9]{22}$").is_match(&sid) {
                continue;
            }
            let label = if js::truthy(&entity["labels"]["fr"]) { js::gs(&entity["labels"]["fr"], "value") } else { js::gs(&entity["labels"]["en"], "value") };
            let words: Vec<String> = norm(&label).split(' ').map(String::from).collect();
            if wanted.is_empty() || !wanted.iter().all(|w| words.iter().any(|x| x == w)) {
                continue;
            }
            return Some(json!({ "type": if typ == Some("album") { "album" } else { "artist" }, "id": sid, "title": label, "score": wanted.len(), "rank": 0 }));
        }
        None
    }

    async fn public_entity(&self, typ: &str, id: &str) -> Option<J> {
        let html = self.fetch_text(&format!("https://open.spotify.com/embed/{typ}/{id}"), None).await;
        let m = crate::re!(r#"(?s)<script id="__NEXT_DATA__" type="application/json">(.*?)</script>"#).captures(&html)?;
        let data: J = serde_json::from_str(&m[1]).ok()?;
        let entity = data["props"]["pageProps"]["state"]["data"]["entity"].clone();
        if !js::truthy(&entity) {
            return None;
        }
        let images = if js::truthy(&entity["coverArt"]["sources"]) { js::arr(&entity["coverArt"]["sources"]) } else { js::arr(&entity["visualIdentity"]["image"]) };
        let cover = images.iter().find(|i| js::truthy(&i["url"])).map(|i| i["url"].clone()).unwrap_or(Value::Null);
        let mut tracks: Vec<J> = js::arr(&entity["trackList"])
            .into_iter()
            .filter(|t| js::gs(t, "uri").starts_with("spotify:track:") && t["isPlayable"] != json!(false))
            .map(|t| json!({ "uri": t["uri"], "title": t["title"], "artist": js::gs(&t, "subtitle"), "duration": (js::num(&t["duration"]).unwrap_or(0.0) / 1000.0).round() as i64 }))
            .collect();
        if tracks.is_empty() && js::gs(&entity, "type") == "track" && js::truthy(&entity["uri"]) {
            let title = if js::truthy(&entity["title"]) { entity["title"].clone() } else { entity["name"].clone() };
            tracks.push(json!({ "uri": entity["uri"], "title": title, "artist": js::gs(&entity, "subtitle"), "duration": (js::num(&entity["duration"]).unwrap_or(0.0) / 1000.0).round() as i64 }));
        }
        let etype = if js::truthy(&entity["type"]) { js::gs(&entity, "type") } else { typ.to_string() };
        let name = if js::truthy(&entity["name"]) { js::gs(&entity, "name") } else { js::gs(&entity, "title") };
        Some(json!({ "type": etype, "name": name, "cover": cover, "tracks": tracks }))
    }

    async fn resolve_public(&self, raw: &str) -> Option<J> {
        let (typ, terms) = Self::public_query(raw);
        if terms.is_empty() {
            return None;
        }
        let page = match self.find_public_page(typ, &terms).await {
            Some(p) => p,
            None => self.find_wikidata_page(typ, &terms).await?,
        };
        let entity = self.public_entity(&js::gs(&page, "type"), &js::gs(&page, "id")).await?;
        let tracks = js::arr(&entity["tracks"]);
        if tracks.is_empty() {
            return None;
        }
        let etype = js::gs(&entity, "type");
        let shuffle = etype == "playlist" || typ == Some("playlist");
        let pool: Vec<J> = tracks.into_iter().take(30).collect();
        let t = if shuffle { pool[js::rand_below(pool.len())].clone() } else { pool[0].clone() };
        Some(json!({
            "uri": t["uri"], "kind": etype, "origin": if etype == "track" { Value::Null } else { entity["name"].clone() },
            "title": t["title"], "artist": t["artist"], "album": if etype == "album" { entity["name"].clone() } else { json!("") },
            "duration": t["duration"], "thumbnail": entity["cover"]
        }))
    }

    async fn play_public(&self, query: &str) -> J {
        if self.installed_path().is_none() {
            return json!({ "ok": false, "reason": "not_installed", "message": "L’application Spotify n’est pas installée sur ce PC." });
        }
        match self.resolve_public(query).await {
            Some(t) => self.play_track(t, true).await,
            None => json!({ "ok": false, "reason": "not_found", "message": format!("rien trouvé pour « {} »", Self::clean_query(query)) }),
        }
    }

    // --- Application Spotify --------------------------------------------------------------------------

    pub fn is_running(&self) -> bool {
        jarvis_native::window::is_running("Spotify.exe")
    }

    pub fn installed_path(&self) -> Option<PathBuf> {
        let store = PathBuf::from(js::env("LOCALAPPDATA").unwrap_or_default()).join("Microsoft").join("WindowsApps").join("Spotify.exe");
        let classic = PathBuf::from(js::env("APPDATA").unwrap_or_default()).join("Spotify").join("Spotify.exe");
        // L'alias du Microsoft Store n'est visible qu'avec symlink_metadata (lstat).
        [store, classic].into_iter().find(|p| std::fs::symlink_metadata(p).is_ok())
    }

    /// Démarre l'application si besoin (minimisée pour la version classique).
    pub async fn ensure_running(&self, visible: bool) -> bool {
        if self.is_running() {
            return true;
        }
        let Some(exe) = self.installed_path() else { return false };
        let s = exe.to_string_lossy().into_owned();
        let ok = if s.to_lowercase().contains("windowsapps") {
            crate::proc::detached("explorer.exe", &[&format!("shell:AppsFolder\\{STORE_APP_ID}")])
        } else if visible {
            crate::proc::detached(&s, &[])
        } else {
            crate::proc::detached(&s, &["--minimized"])
        };
        if !ok {
            return false;
        }
        let until = js::now_ms() + LAUNCH_WAIT_MS;
        while js::now_ms() < until {
            js::sleep_ms(600).await;
            if self.is_running() {
                js::sleep_ms(2500).await;
                return true;
            }
        }
        false
    }

    pub async fn send_to_background(&self) {
        let _ = tokio::task::spawn_blocking(minimize_spotify).await;
    }

    /// Envoie l'adresse à l'application (rundll32 la transmet telle quelle).
    pub fn send_uri(&self, uri: &str) {
        crate::proc::detached("rundll32.exe", &["url.dll,FileProtocolHandler", uri]);
    }

    pub async fn open_search(&self, query: &str) -> J {
        let q = Self::clean_query(query);
        if q.is_empty() {
            return json!({ "ok": false, "reason": "empty", "message": "Je n’ai pas compris ce qu’il faut chercher." });
        }
        if self.installed_path().is_none() {
            return json!({ "ok": false, "reason": "not_installed", "message": "L’application Spotify n’est pas installée sur ce PC." });
        }
        if !self.ensure_running(true).await {
            return json!({ "ok": false, "reason": "launch_failed", "message": "Spotify n’a pas démarré." });
        }
        self.send_uri(&format!("spotify:search:{}", urlencoding::encode(&q)));
        (self.log)(format!("[spotify] recherche ouverte : {q}"));
        json!({ "ok": true, "opened": true, "query": q })
    }

    /// Lecture directe (API Premium), sinon catalogue public, en dernier recours ouverture de la recherche.
    pub async fn play_or_search(&self, query: &str) -> J {
        if self.c("mode") != "direct" && self.is_configured() && !self.premium_required.get() {
            match self.play(query, true).await {
                Ok(res) => {
                    let reason = js::gs(&res, "reason");
                    if js::gb(&res, "ok") || reason == "not_installed" || reason == "launch_failed" {
                        return res;
                    }
                }
                Err(e) => {
                    if e.contains("403") {
                        self.activate_direct_mode();
                    }
                }
            }
        }
        let res = self.play_public(query).await;
        let reason = js::gs(&res, "reason");
        if js::gb(&res, "ok") || reason == "not_installed" || reason == "launch_failed" {
            return res;
        }
        self.open_search(query).await
    }

    pub async fn play(&self, query: &str, discreet: bool) -> Result<J, String> {
        if !self.is_configured() {
            return Ok(json!({ "ok": false, "reason": "not_configured", "message": "Spotify n’est pas encore configuré (Administration → Connecteurs)." }));
        }
        if self.installed_path().is_none() {
            return Ok(json!({ "ok": false, "reason": "not_installed", "message": "L’application Spotify n’est pas installée sur ce PC." }));
        }
        match self.resolve(query).await {
            Ok(t) => Ok(self.play_track(t, discreet).await),
            Err(e) => Ok(json!({ "ok": false, "reason": "not_found", "message": e })),
        }
    }

    async fn play_track(&self, track: J, discreet: bool) -> J {
        if !self.ensure_running(false).await {
            return json!({ "ok": false, "reason": "launch_failed", "message": "Spotify n’a pas démarré." });
        }
        self.send_uri(&js::gs(&track, "uri"));
        if discreet {
            js::spawn(async {
                js::sleep_ms(SETTLE_MS).await;
                let _ = tokio::task::spawn_blocking(minimize_spotify).await;
            });
        }
        let artist = js::gs(&track, "artist");
        (self.log)(format!("[spotify] lecture : {}{}", js::gs(&track, "title"), if artist.is_empty() { String::new() } else { format!(" — {artist}") }));
        js::spread(&json!({ "ok": true }), &track)
    }

    /// Vérifie les identifiants sans rien jouer (bouton « Tester »).
    pub async fn test(&self) -> J {
        if !self.is_configured() {
            return json!({ "ok": false, "message": "Renseigne d’abord le client ID et le secret." });
        }
        let installed = self.installed_path().is_some();
        if let Err(e) = self.access_token().await {
            return json!({ "ok": false, "step": "token", "tokenOk": false, "message": e, "installed": installed });
        }
        match self.search("daft punk", "artist", 1).await {
            Ok(data) => {
                let found = js::arr(&data["artists"]["items"]).into_iter().next();
                json!({ "ok": true, "tokenOk": true, "message": match found { Some(f) => format!("Connexion établie (recherche vérifiée : {}).", js::gs(&f, "name")), None => "Connexion établie.".into() }, "installed": installed })
            }
            Err(e) => json!({ "ok": false, "step": "search", "tokenOk": true, "message": e, "installed": installed }),
        }
    }
}
