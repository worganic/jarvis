// Connecteurs personnels Google (orchestrator/google.js) : Gmail (IMAP/SMTP avec un mot de passe
// d'application) et Agenda (Google Calendar par OAuth, sinon agenda local data/calendar.json). Les
// identifiants restent dans data/google.json (mot de passe chiffré par DPAPI), jamais transmis aux modèles.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use base64::Engine;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::mail_parse;
use crate::js::{self, J};
use crate::secret_box::SecretBox;

const APP_PASSWORD_HELP: &str = "Crée-en un sur myaccount.google.com/apppasswords (la validation en deux étapes doit être activée), puis colle les 16 lettres.";

pub fn decode_mime_words(value: &str) -> String {
    mail_parse::decode_words(value)
}

/// Mot de passe d'application Google : 16 lettres. → Ok(mot de passe) ou Err(explication).
pub fn check_app_password(raw: &str) -> Result<String, String> {
    let password = crate::re!("[\\s\u{a0}\u{200b}-\u{200d}\u{2060}\u{feff}-]+").replace_all(raw, "").into_owned();
    if crate::re!(r"(?i)^[a-z]{16}$").is_match(&password) {
        return Ok(password.to_lowercase());
    }
    let n = password.chars().count();
    let extra = if crate::re!(r"(?i)[^a-z]").is_match(&password) { ", avec des chiffres ou des symboles" } else { "" };
    Err(format!(
        "Ce que tu as collé fait {n} caractère{}{extra} : ce n’est pas un mot de passe d’application. Google en affiche un de 16 lettres minuscules, en 4 groupes de 4 (par exemple « abcd efgh ijkl mnop »), une seule fois, juste après « Créer ». Ton mot de passe Google habituel ne fonctionnera jamais ici.",
        if n > 1 { "s" } else { "" }
    ))
}

/// Refus de connexion Gmail, en clair : (identifiants en cause, texte).
pub fn explain_gmail_error(message: &str) -> (bool, String) {
    if crate::re!(r"(?i)application-specific password required|app(?:lication)?[- ]specific password").is_match(message) {
        return (true, format!("Gmail refuse ton mot de passe habituel : il exige un mot de passe d’application. {APP_PASSWORD_HELP}"));
    }
    if crate::re!(r"(?i)AUTHENTICATIONFAILED|Invalid credentials|Username and Password not accepted|\b535\b").is_match(message) {
        return (true, format!("Gmail refuse ces identifiants (adresse ou mot de passe d’application incorrect, ou révoqué). {APP_PASSWORD_HELP}"));
    }
    if crate::re!(r"(?i)Web login required|WEBALERT|\b534\b").is_match(message) {
        return (true, "Google bloque la connexion : ouvre Gmail dans ton navigateur pour confirmer que c’est bien toi, puis réessaie.".into());
    }
    (false, message.to_string())
}

/// Liste française : « a », « a et b », « a, b et c ».
pub fn list_fr(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} et {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

pub struct GoogleConnector {
    secret_box: Option<Rc<SecretBox>>,
    config_file: PathBuf,
    calendar_file: PathBuf,
    pub config: RefCell<J>,
}

type Cmd = Box<dyn Fn(&str, &str, &str) -> String>;

impl GoogleConnector {
    pub fn new(config_file: PathBuf, calendar_file: PathBuf, secret_box: Option<Rc<SecretBox>>) -> Self {
        let g = Self { secret_box, config_file, calendar_file, config: RefCell::new(json!({})) };
        *g.config.borrow_mut() = g.load_config();
        g
    }

    fn load_config(&self) -> J {
        if let Some(mut raw) = js::read_json(&self.config_file) {
            if js::truthy(&raw["appPasswordEnc"]) {
                if let Some(b) = &self.secret_box {
                    raw["appPassword"] = json!(b.decrypt(&js::gs(&raw, "appPasswordEnc")).unwrap_or_default());
                }
            }
            if let Some(o) = raw.as_object_mut() {
                o.remove("appPasswordEnc");
            }
            return raw;
        }
        json!({ "email": "", "appPassword": "", "oauthToken": "", "calendarId": "primary" })
    }

    pub fn save_config(&self, patch: J) -> J {
        let merged = js::spread(&self.config.borrow(), &patch);
        *self.config.borrow_mut() = merged.clone();
        let mut stored = merged;
        if let (Some(b), true) = (&self.secret_box, js::truthy(&stored["appPassword"])) {
            if let Ok(enc) = b.encrypt(&js::gs(&stored, "appPassword")) {
                stored["appPasswordEnc"] = json!(enc);
                stored.as_object_mut().unwrap().remove("appPassword");
            }
        }
        let _ = js::write_json(&self.config_file, &stored);
        json!({ "ok": true, "message": "Configuration Google enregistrée" })
    }

    fn c(&self, k: &str) -> String {
        js::gs(&self.config.borrow(), k)
    }

    /// Chiffrement DPAPI disponible pour le mot de passe d'application.
    pub fn secret_box_available(&self) -> bool {
        self.secret_box.as_ref().map(|b| b.available()).unwrap_or(false)
    }

    pub fn is_configured(&self) -> bool {
        !self.c("email").is_empty() && (!self.c("appPassword").is_empty() || !self.c("oauthToken").is_empty())
    }

    pub fn missing_setup(&self) -> Vec<String> {
        let mut m = vec![];
        if self.c("email").is_empty() {
            m.push("ton adresse Gmail".to_string());
        }
        if self.c("appPassword").is_empty() && self.c("oauthToken").is_empty() {
            m.push("ton mot de passe d’application Google".to_string());
        }
        m
    }

    fn not_configured(&self, action: &str) -> J {
        let missing = self.missing_setup();
        json!({
            "ok": false, "reason": "not_configured", "setup": { "connector": "gmail", "missing": missing },
            "message": format!("Il me manque {} pour {action}. Je t’ouvre la page de configuration de Gmail pour les renseigner.", list_fr(&missing))
        })
    }

    fn failure(&self, err: &str, prefix: &str) -> J {
        let (auth, text) = explain_gmail_error(err);
        if !auth {
            return json!({ "ok": false, "error": err, "message": format!("{prefix} : {err}") });
        }
        json!({
            "ok": false, "error": err, "reason": "auth", "setup": { "connector": "gmail", "missing": ["un mot de passe d’application valide"] },
            "message": format!("{text} Je t’ouvre la page de configuration de Gmail.")
        })
    }

    pub fn statut(&self) -> J {
        let configured = self.is_configured();
        let email = self.c("email");
        let masked = if email.is_empty() { Value::Null } else { json!(crate::re!(r"(.{2})(.*)(@.*)").replace(&email, "$1***$3")) };
        json!({
            "configure": configured, "email": masked,
            "mode": if !self.c("appPassword").is_empty() { "Mot de passe d’application (IMAP/SMTP)" } else if !self.c("oauthToken").is_empty() { "OAuth2 Google" } else { "Non configuré" },
            "message": if configured { format!("Compte Gmail connecté ({email}).") } else { "Le compte Google n'est pas encore configuré. Pour activer Gmail, génère un mot de passe d'application dans ton compte Google (Sécurité > Validation en deux étapes > Mots de passe des applications) puis dis à Jarvis de le configurer.".to_string() }
        })
    }

    pub async fn google_configurer(&self, p: &J) -> J {
        let email = js::gs(p, "email");
        let pass = js::gs(p, "mot_de_passe_application");
        if email.is_empty() || pass.is_empty() {
            return json!({ "ok": false, "error": "Email et mot de passe d’application requis." });
        }
        let clean = match check_app_password(&pass) {
            Ok(p) => p,
            Err(t) => return json!({ "ok": false, "error": t, "message": t }),
        };
        let clean_email = email.trim().to_lowercase();
        self.save_config(json!({ "email": clean_email, "appPassword": clean }));
        let test = self.mail_derniers(&json!({ "limite": 1 })).await;
        if !js::gb(&test, "ok") {
            let e = if js::truthy(&test["error"]) { js::gs(&test, "error") } else { js::gs(&test, "message") };
            let (_, why) = explain_gmail_error(&e);
            return json!({ "ok": false, "error": why, "message": format!("Configuration échouée : {why}") });
        }
        json!({ "ok": true, "email": clean_email, "message": format!("Compte Google connecté avec succès ({clean_email}).") })
    }

    // --- IMAP (TLS) -------------------------------------------------------------------------------

    async fn execute_imap(&self, commands: Vec<Cmd>) -> Result<String, String> {
        let email = self.c("email");
        let password = crate::re!(r"\s+").replace_all(&self.c("appPassword"), "").into_owned();
        if email.is_empty() || password.is_empty() {
            return Err("Gmail non configuré. Renseigne ton adresse et un mot de passe d'application Google.".into());
        }
        let run = async {
            let mut s = crate::net::tls_connect("imap.gmail.com", 993).await.map_err(|e| format!("Connexion IMAP impossible : {e}"))?;
            let mut buffer = String::new();
            let mut raw = Vec::new();
            let mut chunk = vec![0u8; 65536];
            // Lit jusqu'à une ligne qui satisfait `done` (sur la dernière ligne complète).
            async fn read_until(s: &mut tokio_rustls::client::TlsStream<tokio::net::TcpStream>, raw: &mut Vec<u8>, buffer: &mut String, chunk: &mut [u8], tag: Option<&str>) -> Result<(), String> {
                loop {
                    let text = String::from_utf8_lossy(raw).into_owned();
                    let lines: Vec<&str> = text.split("\r\n").collect();
                    let last = if lines.len() >= 2 { lines[lines.len() - 2] } else { lines.last().copied().unwrap_or("") };
                    match tag {
                        None => {
                            if text.contains("* OK") {
                                *buffer = text;
                                return Ok(());
                            }
                        }
                        Some(t) => {
                            if let Some(c) = crate::re!(r"^(TAG\S*)\s+(OK|NO|BAD)\b").captures(last) {
                                if &c[1] == t {
                                    *buffer = text.clone();
                                    if &c[2] != "OK" {
                                        return Err(format!("Erreur IMAP Gmail : {last}"));
                                    }
                                    return Ok(());
                                }
                            }
                        }
                    }
                    let n = s.read(chunk).await.map_err(|e| format!("Connexion IMAP impossible : {e}"))?;
                    if n == 0 {
                        *buffer = String::from_utf8_lossy(raw).into_owned();
                        return Ok(());
                    }
                    raw.extend_from_slice(&chunk[..n]);
                }
            }
            read_until(&mut s, &mut raw, &mut buffer, &mut chunk, None).await?;
            const TAGS: [&str; 7] = ["TAG1", "TAG2", "TAG3", "TAG4", "TAG5", "TAG6", "TAG7"];
            for (i, cmd) in commands.iter().enumerate() {
                let tag = TAGS.get(i).map(|t| t.to_string()).unwrap_or_else(|| format!("TAG_{i}"));
                let line = cmd(&tag, &email, &password);
                s.write_all(format!("{line}\r\n").as_bytes()).await.map_err(|e| e.to_string())?;
                read_until(&mut s, &mut raw, &mut buffer, &mut chunk, Some(&tag)).await?;
            }
            s.write_all(b"TAG_END LOGOUT\r\n").await.map_err(|e| e.to_string())?;
            let _ = read_until(&mut s, &mut raw, &mut buffer, &mut chunk, Some("TAG_END")).await;
            Ok::<String, String>(buffer)
        };
        match tokio::time::timeout(std::time::Duration::from_secs(15), run).await {
            Ok(r) => r,
            Err(_) => Err("Délai d’attente dépassé avec le serveur Gmail (IMAP timeout)".into()),
        }
    }

    fn parse_header(raw: &str) -> J {
        let block = crate::re!(r"\r?\n[ \t]+").replace_all(raw, " ");
        let get = |name: &str| -> String {
            let re = regex::Regex::new(&format!(r"(?mi)^{name}:[ \t]*(.*?)\r?$")).unwrap();
            re.captures(&block).map(|c| crate::re!(r"\s+").replace_all(&decode_mime_words(&c[1]), " ").trim().to_string()).unwrap_or_default()
        };
        let head200: String = raw.chars().take(200).collect();
        let uid = crate::re!(r"(?i)^\s*\(?\s*UID\s+(\d+)")
            .captures(raw)
            .and_then(|c| c[1].parse::<i64>().ok())
            .or_else(|| crate::re!(r"(?i)\bUID\s+(\d+)").captures(&head200).and_then(|c| c[1].parse::<i64>().ok()));
        let subject = get("Subject");
        let mut v = json!({ "from": get("From"), "subject": if subject.is_empty() { "(Sans objet)".to_string() } else { subject }, "date": get("Date") });
        if let Some(u) = uid {
            v["uid"] = json!(u);
        }
        v
    }

    fn login() -> Cmd {
        Box::new(|tag, email, pass| format!("{tag} LOGIN \"{email}\" \"{pass}\""))
    }

    /// Mail complet (card de lecture) : lecture seule, 400 premiers Ko.
    pub async fn mail_lire(&self, p: &J) -> J {
        if !self.is_configured() {
            return self.not_configured("lire ce mail");
        }
        let id = js::num(&p["uid"]).unwrap_or(0.0);
        if id <= 0.0 || id.fract() != 0.0 {
            return json!({ "ok": false, "message": "Identifiant de mail invalide." });
        }
        let id = id as i64;
        let raw = self
            .execute_imap(vec![Self::login(), Box::new(|t, _, _| format!("{t} EXAMINE INBOX")), Box::new(move |t, _, _| format!("{t} UID FETCH {id} (UID X-GM-MSGID BODY.PEEK[]<0.400000>)"))])
            .await;
        let raw = match raw {
            Ok(r) => r,
            Err(e) => return self.failure(&e, "Lecture du mail impossible"),
        };
        let Some(start) = crate::re!(r"\* \d+ FETCH \(").find(&raw).map(|m| m.start()) else {
            return json!({ "ok": false, "message": "Ce mail n’existe plus dans la boîte de réception." });
        };
        let head: String = raw[start..].chars().take(400).collect();
        let Some(lit) = crate::re!(r"\{(\d+)\}\r\n").captures(&raw[start..]) else {
            return json!({ "ok": false, "message": "Contenu du mail illisible." });
        };
        let whole = lit.get(0).unwrap();
        let body_start = start + whole.end();
        let tag_line = raw.rfind("\r\nTAG3 ");
        let body = match tag_line {
            Some(t) if t > body_start => &raw[body_start..t],
            _ => &raw[body_start..],
        };
        let body = crate::re!(r"\)\s*$").replace(body, "").into_owned();
        let mail = mail_parse::parse_mail(&body);
        let msg_id = crate::re!(r"X-GM-MSGID\s+(\d+)").captures(&head).and_then(|c| c[1].parse::<u128>().ok());
        let mut out = json!({ "ok": true, "uid": id });
        js::assign(&mut out, mail);
        out["tronque"] = json!(lit[1].parse::<i64>().unwrap_or(0) >= 400000);
        out["lien"] = match msg_id {
            Some(m) => json!(format!("https://mail.google.com/mail/u/0/#all/{m:x}")),
            None => Value::Null,
        };
        out
    }

    async fn search_and_fetch(&self, criteria: String, count: usize) -> Result<Option<Vec<J>>, String> {
        let raw = self.execute_imap(vec![Self::login(), Box::new(|t, _, _| format!("{t} SELECT INBOX")), Box::new(move |t, _, _| format!("{t} SEARCH {criteria}"))]).await?;
        let Some(m) = crate::re!(r"\* SEARCH ([\d\s]+)").captures(&raw) else { return Ok(None) };
        let ids: Vec<i64> = m[1].split_whitespace().filter_map(|x| x.parse().ok()).filter(|x| *x != 0).collect();
        if ids.is_empty() {
            return Ok(None);
        }
        let target: Vec<String> = ids[ids.len().saturating_sub(count)..].iter().map(|i| i.to_string()).collect();
        let list = target.join(",");
        let fetched = self
            .execute_imap(vec![Self::login(), Box::new(|t, _, _| format!("{t} SELECT INBOX")), Box::new(move |t, _, _| format!("{t} FETCH {list} (UID BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE)])"))])
            .await?;
        let mut mails = vec![];
        for chunk in crate::re!(r"\* \d+ FETCH").split(&fetched).skip(1) {
            let parsed = Self::parse_header(chunk);
            if js::truthy(&parsed["from"]) || js::truthy(&parsed["subject"]) {
                mails.push(parsed);
            }
        }
        mails.reverse();
        Ok(Some(mails))
    }

    /// Par défaut : mails NON LUS uniquement ; tous: true → tous les mails.
    pub async fn mail_derniers(&self, p: &J) -> J {
        if !self.is_configured() {
            return self.not_configured("lire tes mails");
        }
        let count = js::gn_or(p, "limite", 5.0).clamp(1.0, 20.0) as usize;
        let unread_only = !(p["tous"] == json!(true) || p["tous"] == json!("true"));
        match self.search_and_fetch(if unread_only { "UNSEEN".into() } else { "ALL".into() }, count).await {
            Err(e) => self.failure(&e, "Impossible de relever les mails"),
            Ok(None) => json!({ "ok": true, "mails": [], "nonLus": unread_only, "message": if unread_only { "Aucun mail non lu." } else { "Votre boîte de réception est vide." } }),
            Ok(Some(mails)) => json!({
                "ok": true, "nombre": mails.len(), "nonLus": unread_only, "mails": mails,
                "message": if unread_only { format!("{} derniers emails non lus récupérés.", mails.len()) } else { format!("{} derniers emails récupérés.", mails.len()) }
            }),
        }
    }

    pub async fn mail_chercher(&self, p: &J) -> J {
        if !self.is_configured() {
            return self.not_configured("chercher dans tes mails");
        }
        let requete = js::gs(p, "requete");
        if requete.is_empty() {
            return self.mail_derniers(&json!({ "limite": p["limite"] })).await;
        }
        let safe = crate::re!(r#"["\r\n]"#).replace_all(&requete, "").trim().to_string();
        let count = js::gn_or(p, "limite", 5.0).clamp(1.0, 20.0) as usize;
        match self.search_and_fetch(format!("TEXT \"{safe}\""), count).await {
            Err(e) => self.failure(&e, "Recherche d’emails échouée"),
            Ok(None) => json!({ "ok": true, "mails": [], "message": format!("Aucun email trouvé pour la recherche : \"{requete}\".") }),
            Ok(Some(mails)) => json!({ "ok": true, "nombre": mails.len(), "recherche": requete, "mails": mails, "message": format!("{} emails trouvés pour \"{requete}\".", mails.len()) }),
        }
    }

    // --- SMTP (TLS) -------------------------------------------------------------------------------

    pub async fn mail_envoyer(&self, p: &J) -> Result<J, String> {
        if !self.is_configured() {
            return Ok(self.not_configured("envoyer un mail"));
        }
        let dest = js::gs(p, "destinataire");
        if !dest.contains('@') {
            return Err("Adresse email du destinataire invalide.".into());
        }
        let objet = js::gs(p, "objet");
        let corps = js::gs(p, "corps");
        let email = self.c("email");
        let pass = crate::re!(r"\s+").replace_all(&self.c("appPassword"), "").into_owned();
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s.as_bytes());
        let run = async {
            let mut s = crate::net::tls_connect("smtp.gmail.com", 465).await.map_err(|e| format!("Erreur de transmission SMTP : {e}"))?;
            let mut buf = vec![0u8; 8192];
            async fn expect(s: &mut tokio_rustls::client::TlsStream<tokio::net::TcpStream>, buf: &mut [u8], want: u16) -> Result<(), String> {
                let mut acc = String::new();
                loop {
                    let n = s.read(buf).await.map_err(|e| format!("Erreur de transmission SMTP : {e}"))?;
                    if n == 0 {
                        return Err("Erreur de transmission SMTP : connexion fermée".into());
                    }
                    acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                    // Réponse complète : dernière ligne « 250 texte » (sans tiret).
                    let lines: Vec<&str> = acc.split("\r\n").filter(|l| !l.is_empty()).collect();
                    if let Some(last) = lines.last() {
                        if last.len() >= 4 && &last[3..4] == "-" {
                            continue;
                        }
                    }
                    let code: u16 = acc.get(0..3).and_then(|c| c.parse().ok()).unwrap_or(0);
                    if code >= 400 {
                        return Err(format!("Erreur SMTP ({code}) : {}", acc.trim()));
                    }
                    if code == want {
                        return Ok(());
                    }
                }
            }
            expect(&mut s, &mut buf, 220).await?;
            let steps: Vec<(String, u16)> = vec![
                ("EHLO localhost".into(), 250),
                ("AUTH LOGIN".into(), 334),
                (b64(&email), 334),
                (b64(&pass), 235),
                (format!("MAIL FROM:<{email}>"), 250),
                (format!("RCPT TO:<{dest}>"), 250),
                ("DATA".into(), 354),
            ];
            for (cmd, want) in steps {
                s.write_all(format!("{cmd}\r\n").as_bytes()).await.map_err(|e| e.to_string())?;
                expect(&mut s, &mut buf, want).await?;
            }
            let subject = if objet.is_empty() { "Sans objet".to_string() } else { objet.clone() };
            let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
            let mime = [
                format!("From: {email}"),
                format!("To: {dest}"),
                format!("Subject: =?UTF-8?B?{}?=", b64(&subject)),
                "MIME-Version: 1.0".into(),
                "Content-Type: text/plain; charset=UTF-8".into(),
                "Content-Transfer-Encoding: 8bit".into(),
                format!("Date: {date}"),
                String::new(),
                corps.clone(),
                ".\r\n".into(),
            ]
            .join("\r\n");
            s.write_all(mime.as_bytes()).await.map_err(|e| e.to_string())?;
            expect(&mut s, &mut buf, 250).await?;
            let _ = s.write_all(b"QUIT\r\n").await;
            let _ = s.shutdown().await;
            Ok::<(), String>(())
        };
        match tokio::time::timeout(std::time::Duration::from_secs(15), run).await {
            Err(_) => Err("Délai d’attente dépassé avec le serveur SMTP Gmail".into()),
            Ok(Err(e)) => Err(e),
            Ok(Ok(())) => Ok(json!({ "ok": true, "destinataire": dest, "objet": objet, "message": format!("Email envoyé avec succès à {dest}") })),
        }
    }

    // --- Agenda -----------------------------------------------------------------------------------

    fn load_calendar(&self) -> Vec<J> {
        js::read_json(&self.calendar_file).map(|v| js::arr(&v)).unwrap_or_default()
    }

    fn save_calendar(&self, events: &[J]) {
        let _ = js::write_json(&self.calendar_file, &Value::Array(events.to_vec()));
    }

    fn calendar_url(&self, suffix: &str) -> String {
        let id = if self.c("calendarId").is_empty() { "primary".to_string() } else { self.c("calendarId") };
        format!("https://www.googleapis.com/calendar/v3/calendars/{}/events{suffix}", urlencoding::encode(&id))
    }

    async fn google_events(&self, time_min: &str, time_max: &str) -> Option<Vec<J>> {
        let token = self.c("oauthToken");
        if token.is_empty() {
            return None;
        }
        let url = self.calendar_url(&format!("?timeMin={time_min}&timeMax={time_max}&singleEvents=true&orderBy=startTime"));
        let resp = crate::net::http().get(url).bearer_auth(token).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let data: J = resp.json().await.ok()?;
        Some(
            js::arr(&data["items"])
                .iter()
                .map(|e| {
                    json!({
                        "id": e["id"], "titre": if js::truthy(&e["summary"]) { e["summary"].clone() } else { json!("(Sans titre)") },
                        "debut": if js::truthy(&e["start"]["dateTime"]) { e["start"]["dateTime"].clone() } else { e["start"]["date"].clone() },
                        "fin": if js::truthy(&e["end"]["dateTime"]) { e["end"]["dateTime"].clone() } else { e["end"]["date"].clone() },
                        "lieu": if js::truthy(&e["location"]) { e["location"].clone() } else { Value::Null },
                        "source": "Google Calendar"
                    })
                })
                .collect(),
        )
    }

    /// Rendez-vous entre deux instants (Jarvis Nomade) : Google Agenda, sinon l'agenda local.
    pub async fn agenda_periode(&self, from_ms: i64, to_ms: i64) -> Vec<J> {
        if let Some(items) = self.google_events(&js::iso_of(from_ms), &js::iso_of(to_ms)).await {
            return items;
        }
        let (start, end): (String, String) = (js::iso_of(from_ms).chars().take(10).collect(), js::iso_of(to_ms).chars().take(10).collect());
        self.load_calendar()
            .into_iter()
            .filter(|e| {
                let d: String = (if js::truthy(&e["date"]) { js::gs(e, "date") } else { js::gs(e, "debut") }).chars().take(10).collect();
                d >= start && d <= end
            })
            .collect()
    }

    pub async fn agenda_jour(&self, p: &J) -> J {
        let date = js::gs(p, "date");
        let target_ms = if date.is_empty() { js::now_ms() } else { js::parse_date_ms(&date).unwrap_or_else(js::now_ms) };
        let iso: String = js::iso_of(target_ms).chars().take(10).collect();
        if let Some(items) = self.google_events(&format!("{iso}T00:00:00Z"), &format!("{iso}T23:59:59Z")).await {
            return json!({ "ok": true, "date": iso, "nombre": items.len(), "evenements": items, "message": format!("{} rendez-vous trouvés pour le {iso} sur Google Calendar.", items.len()) });
        }
        let matching: Vec<J> = self
            .load_calendar()
            .into_iter()
            .filter(|e| {
                let d = if js::truthy(&e["date"]) { js::gs(e, "date") } else { js::gs(e, "debut") };
                d.starts_with(&iso)
            })
            .collect();
        let message = if matching.is_empty() {
            format!("Aucun rendez-vous noté pour le {iso}.")
        } else {
            format!("{} événement(s) prévus le {iso} : {}", matching.len(), matching.iter().map(|m| format!("{} {}", js::gs(m, "heure"), js::gs(m, "titre"))).collect::<Vec<_>>().join(", "))
        };
        json!({ "ok": true, "date": iso, "nombre": matching.len(), "evenements": matching, "message": message })
    }

    pub async fn agenda_semaine(&self, _p: &J) -> J {
        let now = js::now_ms();
        let later = now + 7 * 86_400_000;
        let start_iso: String = js::iso_of(now).chars().take(10).collect();
        let end_iso: String = js::iso_of(later).chars().take(10).collect();
        if let Some(items) = self.google_events(&js::iso_of(now), &js::iso_of(later)).await {
            return json!({ "ok": true, "periode": format!("{start_iso} au {end_iso}"), "nombre": items.len(), "evenements": items, "message": format!("{} rendez-vous prévus sur les 7 prochains jours.", items.len()) });
        }
        let matching: Vec<J> = self
            .load_calendar()
            .into_iter()
            .filter(|e| {
                let d: String = (if js::truthy(&e["date"]) { js::gs(e, "date") } else { js::gs(e, "debut") }).chars().take(10).collect();
                d >= start_iso && d <= end_iso
            })
            .collect();
        json!({ "ok": true, "periode": format!("{start_iso} au {end_iso}"), "nombre": matching.len(), "evenements": matching, "message": format!("{} événement(s) notés pour les 7 prochains jours.", matching.len()) })
    }

    pub async fn agenda_creer(&self, p: &J) -> Result<J, String> {
        let titre = js::gs(p, "titre");
        if titre.is_empty() {
            return Err("Titre du rendez-vous obligatoire".into());
        }
        let day = { let d = js::gs(p, "date"); if d.is_empty() { js::iso_now().chars().take(10).collect() } else { d } };
        let heure = { let h = js::gs(p, "heure"); if h.is_empty() { "09:00".to_string() } else { h } };
        let duree = js::gn_or(p, "duree_minutes", 60.0);
        let mut event = json!({
            "id": format!("evt_{}", js::now_ms()), "titre": titre, "date": day, "heure": heure, "duree_minutes": super::config::jnum(duree),
            "description": js::gs(p, "description"), "lieu": js::gs(p, "lieu"), "cree_le": js::iso_now()
        });
        let token = self.c("oauthToken");
        if !token.is_empty() {
            let start_local = format!("{day}T{heure}:00");
            if let Some(start) = js::parse_date_ms(&start_local) {
                let end = start + (duree * 60000.0) as i64;
                let body = json!({
                    "summary": titre, "description": js::gs(p, "description"), "location": js::gs(p, "lieu"),
                    "start": { "dateTime": js::iso_of(start) }, "end": { "dateTime": js::iso_of(end) }
                });
                if let Ok(resp) = crate::net::http().post(self.calendar_url("")).bearer_auth(&token).json(&body).send().await {
                    if resp.status().is_success() {
                        if let Ok(res) = resp.json::<J>().await {
                            event["googleId"] = res["id"].clone();
                        }
                    }
                }
            }
        }
        let mut events = self.load_calendar();
        events.push(event.clone());
        self.save_calendar(&events);
        Ok(json!({ "ok": true, "evenement": event, "message": format!("Rendez-vous ajouté : \"{titre}\" le {day} à {heure}.") }))
    }

    /// Retire un rendez-vous créé par Jarvis (agenda local, et Google Agenda le cas échéant).
    pub async fn agenda_supprimer(&self, p: &J) -> J {
        let id = js::gs(p, "id");
        let events = self.load_calendar();
        let Some(target) = events.iter().find(|e| js::gs(e, "id") == id).cloned() else { return json!({ "ok": false, "message": "Rendez-vous introuvable." }) };
        let token = self.c("oauthToken");
        if js::truthy(&target["googleId"]) && !token.is_empty() {
            let url = self.calendar_url(&format!("/{}", urlencoding::encode(&js::gs(&target, "googleId"))));
            let _ = crate::net::http().delete(url).bearer_auth(token).send().await;
        }
        let left: Vec<J> = events.into_iter().filter(|e| js::gs(e, "id") != id).collect();
        self.save_calendar(&left);
        json!({ "ok": true, "message": format!("Rendez-vous retiré : \"{}\".", js::gs(&target, "titre")) })
    }

    pub fn has(action: &str) -> bool {
        matches!(action, "statut" | "google_statut" | "google_configurer" | "mail_lire" | "mail_derniers" | "mail_chercher" | "mail_envoyer" | "agenda_jour" | "agenda_semaine" | "agenda_creer" | "agenda_supprimer")
    }

    /// Action par son nom (this.google[action] de la v3).
    pub async fn call(&self, action: &str, p: &J) -> Result<J, String> {
        Ok(match action {
            "statut" | "google_statut" => self.statut(),
            "google_configurer" => self.google_configurer(p).await,
            "mail_lire" => self.mail_lire(p).await,
            "mail_derniers" => self.mail_derniers(p).await,
            "mail_chercher" => self.mail_chercher(p).await,
            "mail_envoyer" => self.mail_envoyer(p).await?,
            "agenda_jour" => self.agenda_jour(p).await,
            "agenda_semaine" => self.agenda_semaine(p).await,
            "agenda_creer" => self.agenda_creer(p).await?,
            "agenda_supprimer" => self.agenda_supprimer(p).await,
            _ => return Err(format!("action google inconnue : {action}")),
        })
    }
}
