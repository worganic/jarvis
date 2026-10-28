// Orchestrateur : widgets du HUD, notifications, musique (lecteur interne, Spotify, sessions média
// de Windows), résultats d'outils et connecteurs (orchestrator/index.js, lignes 3723-4135).

use serde_json::{json, Value};

use super::Orchestrator;
use crate::js::{self, J};

/// Copie d'un état média sans sa pochette (trop lourde pour l'IA), avec l'indicateur `pochette`.
fn light(state: &J, pochette: bool) -> J {
    let mut l = state.clone();
    if let Some(m) = l.as_object_mut() {
        m.remove("thumbnail");
    }
    l["pochette"] = json!(pochette);
    l
}

fn media_event(state: &J) -> J {
    let mut ev = json!({ "type": "media" });
    js::assign(&mut ev, state.clone());
    ev
}

impl Orchestrator {
    pub fn show_widget(&self, kind: &str, data: J) {
        let shown = self.cards.register(kind, &data);
        let mut ev = json!({ "type": "widget", "kind": kind });
        js::assign(&mut ev, shown);
        ev["at"] = json!(js::now_ms());
        self.emit(ev);
    }

    /// Notification à l'initiative de Jarvis (veille de Claude Code, rappel, nouveau mail, briefing…) :
    /// parole (en sourdine automatiquement en réunion ou en jeu), card de liste sur le HUD, et relais
    /// vers les autres canaux abonnés (Telegram…). `rows` : lignes de la card (ui/widgets.js).
    pub fn notify(&self, n: J) {
        let text = js::gs(&n, "text");
        if text.is_empty() {
            return;
        }
        let title = Some(js::gs(&n, "title")).filter(|t| !t.is_empty()).unwrap_or_else(|| "Jarvis".into());
        let source = Some(js::gs(&n, "source")).filter(|t| !t.is_empty()).unwrap_or_else(|| "jarvis".into());
        let speak = n["speak"] != json!(false);
        let rows = n["rows"].clone();
        if speak && self.announcer.admit(&source, &text) {
            self.emit(json!({ "type": "say", "text": text, "notification": source }));
        } else if speak {
            // Reportée (heures calmes, sourdine, rafale) : affichée sans être dite, redite plus tard.
            self.emit(json!({ "type": "say", "text": text, "notification": source, "muted": true, "deferred": true }));
        } else {
            self.record("notification", json!({ "source": source, "texte": js::slice_chars(&text, 500) }));
        }
        // card : false → pas de card de liste (les lignes restent pour les canaux abonnés) ; l'information est ailleurs.
        let show_card = n["card"] != json!(false);
        if let Some(list) = rows.as_array().filter(|l| show_card && !l.is_empty()) {
            let count = list.iter().filter(|r| !js::truthy(&r["section"])).count();
            let mut card = json!({
                "title": title, "tool": source, "rows": rows, "count": count,
                "footer": n["footer"], "ttl": n.get("ttl").cloned().unwrap_or(json!(120000)), "notification": source
            });
            if js::truthy(&n["app"]) {
                card["app"] = n["app"].clone();
            }
            self.show_widget("tool", card);
        }
        let payload = json!({ "text": text, "title": title, "source": source, "rows": rows });
        let listeners = self.notify_listeners.borrow().clone();
        for f in listeners {
            f(&payload);
        }
    }

    /// Lance un titre sur Spotify. Rend un état de lecture (badge SPOTIFY sur le HUD) ou un état
    /// d'échec expliqué.
    pub async fn play_on_spotify(&self, query: &str) -> J {
        let wanted = query.trim();
        let fail = |o: &Self, error: String, reason: &str| {
            let state = json!({ "ok": false, "active": false, "app": "SPOTIFY", "error": error, "reason": reason });
            o.emit(media_event(&state));
            state
        };

        // « Lance Spotify », « mets la musique sur Spotify » : aucun titre visé. On ouvre l'application
        // et on appuie sur Lecture par les touches média de Windows (reprise de la dernière écoute).
        if wanted.is_empty() {
            if self.spotify.installed_path().is_none() {
                return fail(self, "L’application Spotify n’est pas installée sur ce PC.".into(), "not_installed");
            }
            if !self.spotify.ensure_running(false).await {
                return fail(self, "Spotify n’a pas démarré.".into(), "launch_failed");
            }
            self.spotify.send_to_background().await;
            // Reprise de la dernière lecture : Spotify fraîchement lancé met quelques secondes à exposer sa session
            // média, la touche Lecture est donc renvoyée jusqu'à ce qu'elle joue (12 s au plus).
            if let Some(smtc) = self.resume_spotify().await {
                self.emit(json!({ "type": "music_control", "action": "stop" }));
                self.emit(media_event(&smtc));
                return light(&smtc, js::truthy(&smtc["thumbnail"]));
            }
            // Rien à reprendre (file vide) : on relance la dernière musique connue (demande réussie, ou titre vu en lecture).
            if let Some(last) = js::read_json(&crate::paths::data_dir().join("spotify-derniere.json")).map(|d| js::gs(&d, "query")).filter(|q| !q.trim().is_empty()) {
                self.record("spotify_reprise_derniere", json!({ "demande": js::slice_chars(&last, 120) }));
                return Box::pin(self.play_on_spotify(&last)).await;
            }
            return fail(self, "Spotify n’a rien à reprendre et je n’ai pas encore de dernière musique en mémoire. Dis-moi ce que tu veux écouter : un titre, un artiste ou une playlist.".into(), "nothing_to_resume");
        }

        // Lecture directe par l'API (Premium) ou par le catalogue public (sans clé) ; en dernier
        // recours, ouverture de la recherche Spotify.
        let res = self.spotify.play_or_search(wanted).await;
        if !js::truthy(&res["ok"]) {
            return fail(self, js::gs(&res, "message"), &js::gs(&res, "reason"));
        }
        if js::truthy(&res["opened"]) {
            // Recherche ouverte : rien ne joue encore. Le lecteur interne se tait quand même.
            self.emit(json!({ "type": "music_control", "action": "stop" }));
            return json!({ "ok": true, "active": false, "opened": true, "app": "SPOTIFY", "query": res["query"] });
        }
        // Le lecteur interne se tait : deux musiques à la fois n'ont aucun sens.
        self.emit(json!({ "type": "music_control", "action": "stop" }));

        // Envoyer l'adresse à Spotify ne prouve pas que la musique joue : on attend que sa session
        // média Windows passe en lecture.
        let playing = self.confirm_spotify_playback(8000).await;
        if playing == Some(Value::Bool(false)) {
            return fail(
                self,
                format!("Spotify a reçu « {} », mais la lecture n’a pas démarré. Vérifie que ton compte est connecté dans l’application.", js::gs(&res, "title")),
                "not_playing",
            );
        }
        // Dernière demande réussie : « mets de la musique » sans précision la rejouera.
        let _ = js::write_json(&crate::paths::data_dir().join("spotify-derniere.json"), &json!({ "query": wanted, "titre": res["title"], "at": js::iso_now() }));
        let origin = if js::truthy(&res["origin"]) { res["origin"].clone() } else { Value::Null };
        let state = json!({
            "ok": true, "active": true, "status": "playing",
            "title": res["title"], "artist": res["artist"], "album": js::gs(&res, "album"),
            "app": "SPOTIFY", "duration": js::gn(&res, "duration").unwrap_or(0.0), "position": 0,
            "canNext": true, "canPrevious": true, "origin": origin, "kind": res["kind"]
        });
        // État réel de la session Spotify s'il est connu (pochette, durée, position), sinon celui
        // du titre envoyé.
        let live = match &playing {
            Some(p) if p.is_object() => {
                let mut l = state.clone();
                js::assign(&mut l, p.clone());
                l["ok"] = json!(true);
                l["app"] = if js::truthy(&p["app"]) { p["app"].clone() } else { json!("SPOTIFY") };
                l["origin"] = state["origin"].clone();
                l["kind"] = state["kind"].clone();
                l
            }
            _ => state.clone(),
        };
        let mut ev = media_event(&live);
        if js::truthy(&state["origin"]) {
            ev["playlist"] = state["origin"].clone();
        }
        ev["thumbnail"] = if js::truthy(&live["thumbnail"]) { live["thumbnail"].clone() } else if js::truthy(&res["thumbnail"]) { res["thumbnail"].clone() } else { Value::Null };
        self.emit(ev);
        let mut r = light(&live, js::truthy(&live["thumbnail"]) || js::truthy(&res["thumbnail"]));
        r["verified"] = json!(matches!(playing, Some(ref p) if p.is_object()));
        r
    }

    /// Touche Lecture envoyée à Spotify, renvoyée toutes les 1,2 s tant que la session n'est pas en lecture (12 s au plus).
    /// Some(état) quand ça joue ; None si Spotify n'a rien à reprendre.
    async fn resume_spotify(&self) -> Option<J> {
        let until = js::now_ms() + 12_000;
        loop {
            let st = self.media.command("play", Some("spotify")).await;
            if js::truthy(&st["active"]) {
                if js::gs(&st, "status") == "playing" {
                    return Some(st);
                }
                if let Some(p) = self.confirm_spotify_playback(3500).await {
                    if p.is_object() {
                        return Some(p);
                    }
                }
            }
            if js::now_ms() >= until {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        }
    }

    /// Dernière musique vue en lecture sur Spotify (lancée par Jarvis ou à la main) : « mets de la musique sur Spotify » sans
    /// précision la rejoue quand Spotify n'a rien à reprendre.
    pub fn remember_spotify_track(&self, ev: &J) {
        let app = js::gs(ev, "app").to_lowercase();
        let (title, artist) = (js::gs(ev, "title"), js::gs(ev, "artist"));
        if !(js::gb(ev, "active") && js::gs(ev, "status") == "playing") || !app.contains("spotify") || title.trim().is_empty() {
            return;
        }
        let query = format!("{title} {artist}").trim().to_string();
        if *self.spotify_last.borrow() == query {
            return;
        }
        *self.spotify_last.borrow_mut() = query.clone();
        let _ = js::write_json(&crate::paths::data_dir().join("spotify-derniere.json"), &json!({ "query": query, "titre": title, "at": js::iso_now() }));
    }

    /// Attend que la session média Windows de Spotify passe en lecture (au plus ~8 s).
    /// Some(état) si elle joue, Some(false) si rien ne joue, None si le contrôle des médias est
    /// indisponible (impossible de vérifier : on ne conclut pas à un échec).
    pub async fn confirm_spotify_playback(&self, timeout_ms: i64) -> Option<J> {
        if !self.media.available() {
            return None;
        }
        let until = js::now_ms() + timeout_ms;
        let mut seen = false;
        while js::now_ms() < until {
            let st = self.media.status(Some("spotify")).await;
            if st["ok"] == json!(false) && !seen {
                return None;
            }
            if js::truthy(&st["active"]) {
                seen = true;
                if js::gs(&st, "status") == "playing" {
                    return Some(st);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        }
        Some(Value::Bool(false))
    }

    /// Webradios du lecteur interne associées à une ambiance (« la playlist bonne nuit »). None :
    /// aucune ne correspond, la playlist demandée n'existe pas dans Jarvis.
    pub fn internal_station(name: &str) -> Option<(&'static str, &'static str, &'static str)> {
        let tp = js::fold_lower(name);
        if crate::re!(r"\b(?:bonne nuit|sommeil|dodo|dormir|calme|detente|relax|nuit|chill|lounge)\b").is_match(&tp) {
            return Some(("Groove Salad Chillout", "SomaFM Ambient Lounge", "Ambiance Lounge & Chillout"));
        }
        if crate::re!(r"\b(?:espace|spatial|etoile|deep space|cosmos|ambian\w*|sci ?-?fi|drone)\b").is_match(&tp) {
            return Some(("Drone Zone Deep Space", "Atmospheric Sci-Fi Soundscape", "Atmosphères Spatiales Sci-Fi"));
        }
        if crate::re!(r"\b(?:tech|pulse|hacker|code|cyber\w*|defcon|synthwave)\b").is_match(&tp) {
            return Some(("DEF CON Tech Pulse", "Electro & Hacker Beats", "Electro & Hacker Beats"));
        }
        if crate::re!(r"\b(?:electro|dance|beats?|blender)\b").is_match(&tp) {
            return Some(("Beat Blender", "Electronic & Deep Beats", "Electronic & Deep Beats"));
        }
        None
    }

    fn internal_state(&self, action: &str, station: Option<(&str, &str, &str)>) -> J {
        let internal = self.internal_music_state.borrow().clone();
        let or = |k: &str, d: &str| Some(js::gs(&internal, k)).filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
        let (mut title, mut artist, mut album) = (or("title", "Nightwave Plaza"), or("artist", "Synthwave & Cyberpunk Radio"), or("album", "Lecteur interne JARVIS"));
        if let Some((t, a, al)) = station {
            (title, artist, album) = (t.into(), a.into(), al.into());
        }
        json!({
            "ok": true,
            "active": action != "stop",
            "status": match action { "pause" => "paused", "stop" => "stopped", _ => "playing" },
            "title": title, "artist": artist, "album": album, "app": "JARVIS AUDIO",
            "duration": js::gn(&internal, "duration").unwrap_or(0.0),
            "position": js::gn(&internal, "position").unwrap_or(0.0),
            "canNext": true, "canPrevious": true
        })
    }

    pub async fn media_action(&self, params: J) -> J {
        let mut action = js::gs(&params, "action");
        if action.is_empty() {
            action = js::gs(&params, "commande");
        }
        let action = if action.is_empty() { "status".to_string() } else { action.to_lowercase() };
        let target = Some(js::gs(&params, "targetPlaylist")).filter(|s| !s.is_empty()).or_else(|| Some(js::gs(&params, "playlist")).filter(|s| !s.is_empty()));
        let mut query = js::gs(&params, "query");
        if query.trim().is_empty() {
            query = js::gs(&params, "requete");
        }
        let query = query.trim().to_string();
        let requested = js::gs(&params, "app").to_lowercase();
        let station = target.as_deref().and_then(Self::internal_station);

        self.show_widget("music", json!({ "open": true }));

        // 1. Spotify joue quand il est demandé, mais aussi pour une playlist nommée qui n'est pas une
        // ambiance du lecteur interne et pour un contenu précis demandé par le modèle.
        let for_spotify = requested.contains("spotify") || (requested != "jarvis" && ((target.is_some() && station.is_none()) || (!query.is_empty() && action == "play")));
        if for_spotify && (action == "play" || !query.is_empty() || target.is_some()) {
            // Échec : le message est renvoyé tel quel, sans basculer en silence sur le lecteur interne.
            let q = if !query.is_empty() { query.clone() } else { target.as_ref().map(|t| format!("playlist {t}")).unwrap_or_default() };
            return self.play_on_spotify(&q).await;
        }

        // 2. Lecture dans le lecteur interne (webradios, fichiers locaux), sans application externe.
        if action == "play" || target.is_some() {
            if let (Some(t), None) = (&target, station) {
                let state = json!({ "ok": false, "active": false, "app": "JARVIS AUDIO", "error": format!("Le lecteur interne n’a pas de playlist « {t} » : demande-la sur Spotify.") });
                self.emit(media_event(&state));
                return state;
            }
            self.emit(json!({ "type": "music_control", "action": "play", "playlist": target }));
            let state = self.internal_state("play", station);
            self.emit(media_event(&state));
            let pochette = js::truthy(&self.internal_music_state.borrow()["thumbnail"]);
            return light(&state, pochette);
        }

        // 2 bis. « Arrête la musique » sans application précisée : tout ce qui joue s'arrête, quelle que soit la source
        // (Spotify, navigateur, lecteur de Windows) et le lecteur interne de Jarvis aussi.
        if action == "pause" && requested.is_empty() {
            let external = self.media.command("pauseall", None).await;
            self.emit(json!({ "type": "music_control", "action": "pause" }));
            let state = if js::truthy(&external["active"]) { external } else { self.internal_state("pause", None) };
            self.emit(media_event(&state));
            let pochette = js::truthy(&state["thumbnail"]);
            return light(&state, pochette);
        }

        // 3. Contrôles (pause, next, previous, status) : une session média externe active reçoit la
        // commande. Application précisée : c'est elle, et elle seule, qui est pilotée.
        if requested != "jarvis" {
            let cmd = if action == "status" { "status" } else { action.as_str() };
            let app = Some(requested.as_str()).filter(|a| !a.is_empty());
            let state = self.media.command(cmd, app).await;
            if js::truthy(&state["active"]) {
                self.emit(media_event(&state));
                return light(&state, js::truthy(&state["thumbnail"]));
            }
        }

        // Application externe visée mais absente : le lecteur interne n'a pas à répondre à sa place.
        if !requested.is_empty() && requested != "jarvis" {
            let name = if requested.contains("spotify") { "Spotify".to_string() } else { requested.clone() };
            let failed = json!({ "ok": false, "active": false, "app": name.to_uppercase(), "error": format!("{name} ne joue rien pour le moment.") });
            self.emit(media_event(&failed));
            return failed;
        }

        // 4. Sinon, on pilote le lecteur interne de JARVIS.
        self.emit(json!({ "type": "music_control", "action": action }));
        let state = self.internal_state(&action, None);
        self.emit(media_event(&state));
        let pochette = js::truthy(&self.internal_music_state.borrow()["thumbnail"]);
        light(&state, pochette)
    }

    /// Résultat d'un outil exécuté par l'orchestrateur (actions Windows et Google).
    pub fn on_tool_result(&self, tool: &str, params: &J, result: &J) {
        if !js::truthy(result) {
            return;
        }
        // Échec (compte non configuré, identifiants refusés…) : le widget dit pourquoi.
        if js::truthy(&result["error"]) || result["ok"] == json!(false) {
            // Informations manquantes ou refusées : la page de configuration s'ouvre d'elle-même.
            if js::truthy(&result["setup"]) {
                self.request_setup(result["setup"].clone(), tool);
            }
            let kind = if tool.contains("mail") {
                Some("mail")
            } else if tool.contains("agenda") {
                Some("agenda")
            } else {
                None
            };
            let message = js::gs(result, "message");
            let setup = js::truthy(&result["setup"]) || crate::re!(r"(?i)non configur").is_match(&message);
            let footer = if setup { "→ Admin (F10) → Connecteurs → Gmail intégré".to_string() } else { String::new() };
            let why = if !message.is_empty() {
                message
            } else if js::truthy(&result["error"]) {
                js::gs(result, "error")
            } else {
                "échec".into()
            };
            match kind {
                Some("mail") => self.show_widget("mail", json!({ "title": "Gmail", "rows": [], "empty": format!("⚠ {why}"), "footer": footer })),
                Some(k) => self.show_widget(k, json!({ "title": "Agenda", "rows": [], "empty": format!("⚠ {why}"), "footer": footer })),
                None => {}
            }
            return;
        }
        if tool == "mcp__google__mail_derniers" || tool == "mcp__google__mail_chercher" {
            // Mêmes lignes que la card « Nouveau mail » (mail_watch.rs › mail_row).
            let mails: Vec<J> = js::arr(&result["mails"]).into_iter().take(8).collect();
            self.mails.remember(&mails); // numéros de la card : « ouvre le mail 3 »
            let non_lus = js::truthy(&result["nonLus"]);
            let title = if tool.ends_with("chercher") {
                format!("Recherche : {}", js::gs(params, "requete"))
            } else if non_lus {
                "Mails non lus".into()
            } else {
                "Derniers emails".into()
            };
            let rows: Vec<J> = mails.iter().enumerate().map(|(i, m)| super::mail_watch::mail_row(m, Some(i + 1))).collect();
            self.show_widget(
                "mail",
                json!({
                    "title": title, "rows": rows, "count": mails.len(),
                    "empty": if non_lus { "Aucun mail non lu." } else { "Aucun email." },
                    "footer": if mails.is_empty() { "" } else { super::mail_watch::MAIL_FOOTER }
                }),
            );
        } else if tool == "mcp__google__agenda_jour" || tool == "mcp__google__agenda_semaine" {
            // Rendez-vous numérotés (3 au plus) et bouton Google Agenda ; 🔔 : rappel 15 minutes avant.
            let items = super::list_cards::ListCards::agenda_items(&js::arr(&result["evenements"]));
            let date = Some(js::gs(result, "date")).filter(|d| !d.is_empty()).unwrap_or_else(|| "jour".into());
            let title = if tool.ends_with("semaine") { "Agenda des 7 jours".to_string() } else { format!("Agenda du {date}") };
            self.list_cards.agenda_card(title, &items);
        }
    }

    /// Un connecteur n'a pas ce qu'il lui faut : Jarvis ouvre l'administration directement sur son
    /// formulaire (« mcp#gmail »). Une fois par minute et par connecteur au plus.
    pub fn request_setup(&self, setup: J, origin: &str) -> bool {
        let connector = js::gs(&setup, "connector");
        if connector.is_empty() {
            return false;
        }
        let now = js::now_ms();
        {
            let mut opened = self.setup_opened_at.borrow_mut();
            if now - opened.get(&connector).copied().unwrap_or(0) < 60000 {
                return false;
            }
            opened.insert(connector.clone(), now);
        }
        self.emit(json!({ "type": "open_admin", "section": format!("mcp#{connector}") }));
        let missing = if setup["missing"].is_array() { setup["missing"].clone() } else { json!([]) };
        self.record("configuration_demandee", json!({ "connecteur": connector, "manque": missing, "origine": origin }));
        true
    }

    /// Appels d'outils des connecteurs externes vus dans le flux de Claude Code : le widget du
    /// connecteur s'ouvre avec l'action, puis se remplit avec le résultat.
    pub fn on_connector_tool_use(&self, task_id: &str, part: &J) {
        let Some(info) = self.mcp_registry.describe_tool(&js::gs(part, "name")) else { return };
        let input = if part["input"].is_object() { part["input"].clone() } else { json!({}) };
        let mut use_ = info.clone();
        use_["input"] = input.clone();
        use_["task"] = json!(task_id);
        use_["at"] = json!(js::now_ms());
        {
            let mut uses = self.tool_uses.borrow_mut();
            uses.insert(js::gs(part, "id"), use_);
            if uses.len() > 200 {
                if let Some(oldest) = uses.iter().min_by_key(|(_, v)| v["at"].as_i64().unwrap_or(0)).map(|(k, _)| k.clone()) {
                    uses.remove(&oldest);
                }
            }
        }
        let widget = js::gs(&info, "widget");
        if widget == "music" {
            self.show_widget("music", json!({ "open": true }));
            self.media.set_keep_alive(true);
        } else {
            self.show_widget(
                if widget.is_empty() { "tool" } else { &widget },
                json!({ "title": info["name"], "tool": info["tool"], "input": input, "pending": true, "id": part["id"] }),
            );
        }
    }

    pub fn on_connector_tool_result(&self, part: &J) {
        let id = js::gs(part, "tool_use_id");
        let Some(use_) = self.tool_uses.borrow_mut().remove(&id) else { return };
        let text = match &part["content"] {
            Value::Array(l) => l.iter().filter(|c| js::gs(c, "type") == "text").map(|c| js::gs(c, "text")).collect::<Vec<_>>().join("\n"),
            other => js::s(other),
        };
        let widget = js::gs(&use_, "widget");
        if widget == "music" {
            // Spotify vient d'agir : la session média de Windows donnera titre et pochette.
            let media = self.media.clone();
            js::set_timeout(800, move || {
                js::spawn(async move {
                    let _ = media.status(None).await;
                });
            });
            return;
        }
        self.show_widget(
            if widget.is_empty() { "tool" } else { &widget },
            json!({
                "title": use_["name"], "tool": use_["tool"], "input": use_["input"], "id": id,
                "result": js::slice_chars(&text, 4000), "error": js::truthy(&part["is_error"])
            }),
        );
    }
}
