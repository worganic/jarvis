// Routeur d'intentions local (orchestrator/router.js) : répond en moins de 200 ms, sans IA, aux demandes
// déterministes du quotidien (heure, date, volume, minuteur, musique, applications connues, réglages…).
// Les compétences locales (mails, agenda, météo, calcul, web, fenêtres, fichiers…) sont dans skill_intents.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, Local, Timelike};
use serde_json::{json, Value};

use super::intent::Intent;
use super::prefilter::{detect_noise, normalize_request, small_talk, trim_end};
use super::{OrchRef, Orchestrator, Up};
use crate::js::{self, Timer, J};

pub fn fold(text: &str) -> String {
    let t = js::fold_lower(text);
    let t = crate::re!(r"['’\-?!,;:]").replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

const TIME_ASK: &str = "(?:(?:dis|donne|rappelle)(?: moi)? |(?:tu sais|sais tu|tu connais|connais tu|tu as|as tu|t as|tu aurais|aurais tu|tu peux me dire|peux tu me dire|tu pourrais me dire) )?";
const TIME_TAIL: &str = "(?: (?:maintenant|actuellement|exactement|precisement|la|en ce moment|donc|au juste|pile|s il te plait|s il vous plait|stp|svp|jarvis|merci|please))*";

/// « Quelle heure est-il ? » et ses variantes, phrase entière (l'heure d'un événement n'en est pas une).
pub fn is_current_time_question(text: &str) -> bool {
    static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = R.get_or_init(|| {
        regex::Regex::new(&format!(
            "^{TIME_ASK}(?:quelle heure(?: (?:est il|il est|est ce|c est|on a|avons nous|il fait|ca fait))?|(?:quel|quelle) est l heure|il est quelle heure|c est quelle heure|on a quelle heure|l heure(?: qu il est| actuelle| exacte)?){TIME_TAIL}$"
        ))
        .unwrap()
    });
    let t = crate::re!(r"[.…]+").replace_all(text, " ");
    re.is_match(crate::re!(r"\s+").replace_all(&t, " ").trim())
}

fn it<F, Fu>(o: &OrchRef, kind: &str, label: &str, detail: &str, f: F) -> Intent
where
    F: Fn(Rc<Orchestrator>) -> Fu + 'static,
    Fu: std::future::Future<Output = J> + 'static,
{
    let o = o.clone();
    let f = Rc::new(f);
    Intent::new(kind, label, detail, move || {
        let (o, f) = (o.clone(), f.clone());
        async move { f(o.up()).await }
    })
}

pub struct Router {
    o: OrchRef,
    log: Rc<dyn Fn(String)>,
    timers: RefCell<HashMap<String, Timer>>,
    timer_counter: Cell<u64>,
}

impl Router {
    pub fn new(o: OrchRef, log: Rc<dyn Fn(String)>) -> Self {
        Self { o, log, timers: RefCell::new(HashMap::new()), timer_counter: Cell::new(0) }
    }

    pub fn orch(&self) -> &OrchRef {
        &self.o
    }

    fn music_intent(&self, text: &str) -> Option<Intent> {
        let t = crate::re!(r"[.…]+").replace_all(text, " ");
        let t = crate::re!(r"\s*(?:s il te plait|s il vous plait|stp|merci)$").replace(t.trim(), "").trim().to_string();
        const MEDIA: &str = "(?:chanson|musique|titre|morceau|piste)";
        let is_spotify = crate::re!(r"(?i)\b(?:spotify|spootify|spotifi)\b").is_match(&t);
        let mut action: Option<&str> = None;
        let mut playlist: Option<String> = None;
        let mut query: Option<String> = None;
        let mut repeat: u32 = 1; // « passe trois chansons »
        static NEXT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        static PREV: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let next = NEXT.get_or_init(|| regex::Regex::new(&format!(r"^(?:{MEDIA} suivante?|suivante?|next|zappe|passe (?:a la )?{MEDIA} suivante?|change de {MEDIA})(?:\s+(?:sur\s+)?(?:spotify|spootify|spotifi))?$")).unwrap());
        let prev = PREV.get_or_init(|| regex::Regex::new(&format!(r"^(?:{MEDIA} precedente?|precedente?|previous|(?:reviens|retourne) (?:a la )?{MEDIA} (?:precedente|d avant)|remets la {MEDIA} precedente)(?:\s+(?:sur\s+)?(?:spotify|spootify|spotifi))?$")).unwrap());
        let pl = crate::re!(r"(?i)^(?:(?:je\s+(?:veux|voudrais)|j\s+aimerais)\s+(?:ecouter\s+)?)?(?:(?:lance|mets?|joue|ecoute|ecouter|passe|remets|relance)\s+(?:moi\s+)?)?(?:(?:de\s+la\s+|la\s+|dla\s+|une\s+|ma\s+)?(?:playlist|playliste)|(?:de\s+la\s+|la\s+)?musique\s+(?:sur\s+(?:spotify|spootify|spotifi)\s+)?(?:la\s+|dla\s+|ma\s+)?(?:playlist|playliste))\s+(?:de\s+|sur\s+|intitulee\s+)?(.+?)(?:\s+(?:sur|avec|dans)\s+(?:spotify|spootify|spotifi))?$").captures(&t);
        if let Some(m) = pl.filter(|m| !crate::re!(r"(?i)^(?:spotify|spootify|spotifi)$").is_match(m[1].trim())) {
            playlist = Some(crate::re!(r"(?i)^(?:la|le|les|de|d|du|des)\s+").replace(m[1].trim(), "").trim().to_string());
            action = Some("play");
        } else if crate::re!(r"^(?:(?:(?:mets|lance|joue|remets|relance|reprends|ouvre|ecoute|ecouter)\s+)?(?:la |de la |un peu de )?musique|(?:mets|lance|joue|remets|relance|reprends|ouvre|ecoute|ecouter)\s+musique|reprends la lecture|play|lecture)(?: (?:sur|avec|dans) (?:spotify|spootify|spotifi|le lecteur))?$").is_match(&t)
            || crate::re!(r"^(?:(?:lance|mets|joue|ouvre|reprends|demarre|active|ecoute|ecouter)\s+(?:sur\s+)?(?:spotify|spootify|spotifi)|(?:spotify|spootify|spotifi)(?:\s+(?:play|lecture))?)$").is_match(&t)
        {
            action = Some("play");
        } else if crate::re!(r"^(?:(?:mets|met)\s+(?:la musique\s+)?en pause|mets pause|pause(?: la musique)?|(?:arrete|coupe|stoppe|stop) (?:la musique|spotify|spootify|spotifi)|(?:pause|stop)\s+(?:spotify|spootify|spotifi))$").is_match(&t) {
            action = Some("pause");
        } else if let Some(m) = crate::re!(r"^(?:passe|saute|zappe)\s+(\d{1,2}|deux|trois|quatre|cinq|six)\s+(?:chansons|titres|morceaux|musiques|pistes)(?:\s+(?:sur\s+)?(?:spotify|spootify|spotifi))?$").captures(&t) {
            repeat = match &m[1] { "deux" => 2, "trois" => 3, "quatre" => 4, "cinq" => 5, "six" => 6, n => n.parse().unwrap_or(1) }.clamp(1, 10);
            action = Some("next");
        } else if crate::re!(r"^(?:recommence|rejoue|relance|remets)\s+(?:la |cette |le )?(?:chanson|musique|titre|morceau)(?:\s+depuis le debut)?$").is_match(&t) {
            action = Some("previous"); // le lecteur revient au début du titre en cours s'il est entamé
        } else if next.is_match(&t) {
            action = Some("next");
        } else if prev.is_match(&t) {
            action = Some("previous");
        } else if crate::re!(r"\b(?:qu est ce qu on ecoute|c est quoi (?:cette|la) (?:chanson|musique)|quelle est (?:cette|la) (?:chanson|musique)|quel est ce (?:titre|morceau)|qui chante)\b").is_match(&t)
            || crate::re!(r"^(?:affiche|montre|ouvre) (?:le )?(?:lecteur|widget) (?:de )?(?:musique|spotify|spootify)$").is_match(&t)
        {
            action = Some("status");
        } else if is_spotify {
            if let Some(m) = crate::re!(r"(?i)^(?:mets?|joue|lance|passe|balance|ecoute|ecouter)\s+(?:moi\s+)?(.+?)\s+sur\s+(?:spotify|spootify|spotifi)$").captures(&t) {
                if !crate::re!(r"(?i)^(?:la |de la |un peu de )?musique$").is_match(m[1].trim()) {
                    query = Some(m[1].trim().to_string());
                    action = Some("play");
                }
            }
        }
        let action = action?;
        let label = match action {
            "play" => match (&query, &playlist) {
                (Some(q), _) => format!("Spotify · {q}"),
                (None, Some(p)) => format!("Playlist « {p} »"),
                _ => "Lecture Musique".into(),
            },
            "pause" => "Pause Musique".into(),
            "next" => "Suivant".into(),
            "previous" => "Précédent".into(),
            _ => "Musique".into(),
        };
        let kind = if query.is_some() { "music_spotify".to_string() } else if playlist.is_some() { "music_playlist".into() } else { format!("music_{action}") };
        let detail = match (&query, &playlist) {
            (Some(q), _) => format!("Spotify : {q}"),
            (None, Some(p)) => format!("playlist {p}"),
            _ => "session Musique".into(),
        };
        let lbl = label.clone();
        Some(it(&self.o, &kind, &label, &detail, move |o| {
            let (playlist, query, label) = (playlist.clone(), query.clone(), lbl.clone());
            async move {
                for _ in 1..repeat {
                    let _ = o.media_action(json!({ "action": action, "app": if is_spotify { json!("spotify") } else { Value::Null } })).await;
                    js::sleep_ms(450).await;
                }
                let state = o.media_action(json!({ "action": action, "targetPlaylist": playlist, "query": query, "app": if is_spotify { json!("spotify") } else { Value::Null } })).await;
                if !js::truthy(&state) || state["ok"] == json!(false) {
                    let why = js::gs(&state, "error");
                    let on_spotify = crate::re!(r"(?i)spotify").is_match(&js::gs(&state, "app"));
                    return json!({
                        "say": if why.is_empty() { "Je n'arrive pas à lancer la musique.".to_string() } else { format!("Je n'ai pas pu lancer {}. {why}", if on_spotify { "Spotify" } else { "la musique" }) },
                        "display": if why.is_empty() { "Musique indisponible".to_string() } else { why }
                    });
                }
                if js::gb(&state, "opened") {
                    let q = js::gs(&state, "query");
                    return json!({ "say": format!("J'ai ouvert la recherche Spotify pour « {q} »."), "display": format!("Spotify · recherche « {q} »") });
                }
                let title = js::gs(&state, "title");
                let artist = js::gs(&state, "artist");
                if action == "status" {
                    if !js::gb(&state, "active") || title.is_empty() {
                        return json!({ "say": "Aucune musique en cours.", "display": "Musique inactive" });
                    }
                    return json!({ "say": super::media::describe_track(&state), "display": format!("{title} — {artist}") });
                }
                if !js::gb(&state, "active") {
                    return json!({ "say": "Le lecteur de musique n'est pas actif.", "display": "Musique inactive" });
                }
                if action == "play" && crate::re!(r"(?i)spotify").is_match(&js::gs(&state, "app")) {
                    let origin = js::gs(&state, "origin");
                    let what = if origin.is_empty() {
                        if title.is_empty() { "la musique".to_string() } else { title.clone() }
                    } else {
                        match js::gs(&state, "kind").as_str() {
                            "playlist" => format!("la playlist {origin}"),
                            "album" => format!("l'album {origin}"),
                            _ => origin,
                        }
                    };
                    return json!({ "say": format!("C'est parti sur Spotify : {what}."), "display": format!("Spotify · {title}{}", if artist.is_empty() { String::new() } else { format!(" — {artist}") }) });
                }
                json!({ "say": null, "display": if title.is_empty() { label } else { format!("{title} — {artist}") } })
            }
        }))
    }

    /// Intention locale, ou None (repli vers l'IA). from_voice : demande venue du micro.
    pub fn route(&self, raw: &str, from_voice: bool) -> Option<Intent> {
        let folded = trim_end(&fold(raw));
        if let Some(noise) = detect_noise(&folded) {
            return Some(Self::ignored_intent(noise));
        }
        if let Some(m) = self.match_multi(&folded, raw) {
            return Some(m);
        }
        if let Some(d) = self.match_(&folded, raw) {
            return Some(d);
        }
        let normalized = normalize_request(&folded);
        if !normalized.is_empty() && normalized != folded {
            if let Some(m) = self.match_multi(&normalized, raw) {
                return Some(m);
            }
            if let Some(f) = self.match_(&normalized, raw) {
                return Some(f);
            }
        }
        let a = self.o.up().addressing_forms();
        let talk = small_talk(&folded, &a).or_else(|| if normalized.is_empty() { None } else { small_talk(&normalized, &a) });
        if let Some(talk) = talk {
            let say = talk["say"].clone();
            let display = if say.is_null() { json!("") } else { say.clone() };
            return Some(Intent::now(&format!("small_talk_{}", js::gs(&talk, "kind")), "Politesse", "réponse locale", json!({ "say": say, "display": display })).quiet());
        }
        if from_voice {
            let base = if normalized.is_empty() { folded.clone() } else { normalized.clone() };
            let words_s = trim_end(&base);
            let count = words_s.split(' ').filter(|w| !w.is_empty()).count();
            let intentional = crate::re!(r"^(?:ouvre|lance|mets|cherche|recherche|trouve|ferme|affiche|montre|lis|dis|joue|envoie|ecris|cree|ajoute|note|rappelle|appelle|verifie|analyse|installe|genere|fais|donne|quel|quelle|quels|quelles|qui|quoi|ou|comment|pourquoi|combien|quand|c est quoi|explique|raconte|traduis|calcule|resume|aide|meteo|blague)\b");
            let joined = words_s.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
            if count <= 2 && !intentional.is_match(&joined) {
                let mut i = Intent::now("ignored", "Demande ignorée", "fragment trop court", json!({ "say": "Pardon, je n'ai pas bien compris.", "display": "Demande incomplète" })).ignored("fragment trop court");
                i.detail = "fragment trop court".into();
                return Some(i);
            }
        }
        None
    }

    /// Chaînage local : « baisse le son et lance la musique », exécuté si chaque partie est locale.
    pub fn match_multi(&self, folded: &str, _raw: &str) -> Option<Intent> {
        let sep = crate::re!(r"(?i)\s+(?:et\s+puis|puis|ensuite|apres\s+quoi|et)\s+");
        if !sep.is_match(folded) {
            return None;
        }
        let parts: Vec<String> = sep.split(folded).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
        if parts.len() < 2 {
            return None;
        }
        let mut intents: Vec<Intent> = vec![];
        for part in &parts {
            if let Some(d) = self.match_(part, part).filter(|i| !i.ignored) {
                intents.push(d);
                continue;
            }
            let norm = normalize_request(part);
            match (!norm.is_empty()).then(|| self.match_(&norm, part)).flatten().filter(|i| !i.ignored) {
                Some(f) => intents.push(f),
                None => return None,
            }
        }
        // « ouvre Paint et dessine un mouton » : le dessin ouvre déjà Paint (covers).
        let covered: Vec<String> = intents.iter().flat_map(|i| i.covers.clone()).collect();
        let steps: Vec<Intent> = intents.into_iter().filter(|i| !(i.kind == "open_app" && covered.contains(&i.detail))).collect();
        if steps.len() == 1 {
            return steps.into_iter().next();
        }
        let labels = steps.iter().map(|i| i.label.clone()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" + ");
        let details = steps.iter().map(|i| if i.detail.is_empty() { i.label.clone() } else { i.detail.clone() }).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" puis ");
        let steps = Rc::new(steps);
        let labels2 = labels.clone();
        let mut composite = Intent::new("composite_intent", if labels.is_empty() { "Actions combinées" } else { &labels }, if details.is_empty() { "enchaînement local" } else { &details }, move || {
            let (steps, labels) = (steps.clone(), labels2.clone());
            async move {
                let mut said = vec![];
                let mut shown = vec![];
                for s in steps.iter() {
                    let r = s.run().await;
                    if js::truthy(&r["say"]) {
                        said.push(js::gs(&r, "say"));
                    }
                    if js::truthy(&r["display"]) {
                        shown.push(js::gs(&r, "display"));
                    }
                }
                json!({ "say": if said.is_empty() { Value::Null } else { json!(said.join(" ")) }, "display": if shown.is_empty() { labels } else { shown.join(" · ") } })
            }
        });
        composite.composite = true;
        Some(composite)
    }

    pub fn ignored_intent(reason: &str) -> Intent {
        Intent::now("ignored", "Demande ignorée", reason, json!({ "say": null, "display": "" })).ignored(reason)
    }

    /// Reconnaissance sur un texte déjà replié par fold().
    pub fn match_(&self, text: &str, raw: &str) -> Option<Intent> {
        let o = &self.o;
        // 0. Administration
        if crate::re!(r"^/(?:admin|config|configuration|parametres|reglages)$").is_match(text)
            || (crate::re!(r"\b(?:ouvre|affiche|montre|va dans|lance)\s+(?:la |les |l |ta |tes |ton )?(?:page de |panneau de |fenetre de )?(?:configuration|parametres|reglages|administration|admin)\b").is_match(text)
                && !crate::re!(r"\b(?:windows|systeme|son|audio|affichage|ecran|reseau|wifi|bluetooth)\b").is_match(text))
        {
            return Some(it(o, "open_admin", "Administration", "ouverture de la configuration", |o| async move {
                o.emit(json!({ "type": "open_admin" }));
                json!({ "say": "Voici la configuration.", "display": "Administration" })
            }));
        }
        // Mes données : archive .jarvis sans mot de passe (secrets des connecteurs retirés) ; la restauration, qui
        // remplace tout, se fait dans l'administration (aperçu, mot de passe, code de validation).
        if crate::re!(r"^(?:exporte|sauvegarde|archive|fais une (?:sauvegarde|archive|copie) de)(?: moi)? (?:toutes |tout )?(?:mes|les) donnees(?: de jarvis)?$").is_match(text) {
            return Some(it(o, "data_export", "Exporter mes données", "archive .jarvis dans le dossier des sauvegardes", |o| async move {
                let (data, out) = (o.data_dir.clone(), crate::backup::destination(&o.config.get("backup")).join(crate::transfer::default_name()));
                let target = out.clone();
                let done = tokio::task::spawn_blocking(move || crate::transfer::export(&data, &target, None, &|_| Ok(String::new()))).await;
                match done {
                    Ok(Ok(r)) => {
                        o.record("donnees_exportees", json!({ "fichiers": r.files, "octets": r.bytes, "secrets": r.secrets, "chiffre": false, "origine": "voix" }));
                        json!({
                            "say": format!("C'est fait : {} fichiers dans le dossier des sauvegardes. Pour y inclure les mots de passe des connecteurs, exporte depuis l'administration, avec un mot de passe.", r.files),
                            "display": out.to_string_lossy(),
                        })
                    }
                    Ok(Err(e)) => json!({ "say": "Je n'ai pas pu créer l'archive.", "display": e }),
                    Err(e) => json!({ "say": "Je n'ai pas pu créer l'archive.", "display": e.to_string() }),
                }
            }));
        }
        if crate::re!(r"^(?:restaure|importe|recupere|remets)(?: moi)? (?:toutes |tout )?(?:mes|les) donnees(?: de jarvis)?(?: (?:depuis|a partir d) (?:une|l|mon) (?:archive|sauvegarde))?$").is_match(text) {
            return Some(it(o, "data_import", "Restaurer mes données", "administration › Mes données", |o| async move {
                o.emit(json!({ "type": "open_admin", "section": "donnees" }));
                json!({ "say": "Choisis l'archive à restaurer dans l'administration.", "display": "Administration › Mes données" })
            }));
        }
        // Jarvis Réseau : « dis à Léa que je rentre », « envoie un message à Léa : … » — seulement vers un ami accepté
        // (sinon la phrase suit son chemin habituel). Le texte est repris de la phrase brute (accents, majuscules).
        if crate::re!(r"^(?:dis|envoie(?: un message)?|ecris|ecrit)(?: moi)? a \S+").is_match(text) {
            let raw_re = crate::re!(r"(?i)^\s*(?:dis|envoie(?: un message)?|[ée]cris|[ée]crit)\s+(?:à|a)\s+(\S+?)[\s,:]+(?:que\s+|qu'|qu’)?(.+?)\s*$");
            if let Some(m) = raw_re.captures(raw) {
                if let Some((id, name)) = o.up().reseau.friend_by_name(&m[1]) {
                    let body = m[2].trim().to_string();
                    return Some(it(o, "reseau_message", "Message à un ami", "Jarvis Réseau, chiffré de bout en bout", move |o| {
                        let (body, name) = (body.clone(), name.clone());
                        async move {
                        let r = o.reseau.send_message(id, &body).await;
                        if js::gb(&r, "ok") {
                            json!({ "say": format!("C'est envoyé à {name}."), "display": format!("→ {name} : {body}") })
                        } else {
                            json!({ "say": format!("Je n'ai pas pu l'envoyer à {name}."), "display": js::gs(&r, "error") })
                        }
                        }
                    }));
                }
            }
        }
        if let Some(m) = self.music_intent(text) {
            return Some(m);
        }
        if let Some(s) = super::skill_intents::skill_intent(self, text, raw) {
            return Some(s);
        }
        // 1. Heure
        if is_current_time_question(text) {
            return Some(it(o, "time", "Heure locale", "horloge système", |_| async move {
                let now = Local::now();
                let (h, m) = (now.hour(), now.minute());
                json!({ "say": format!("Il est {h} heure{} {m:02}.", if h > 1 { "s" } else { "" }), "display": format!("{h}:{m:02}") })
            }));
        }
        // 2. Date
        if crate::re!(r"\b(quel jour (on est|sommes nous|est il))\b").is_match(text) || crate::re!(r"\bquelle est la date\b").is_match(text) || crate::re!(r"\bla date d aujourd hui\b").is_match(text) || text == "la date" {
            return Some(it(o, "date", "Date du jour", "calendrier système", |o| async move {
                let now = Local::now();
                let bday = o.memory.is_birthday_today();
                let name = o.memory.get_user_name();
                let formatted = format!("{} {} {} {}", js::JOURS[now.weekday().num_days_from_sunday() as usize], now.day(), js::MOIS[now.month0() as usize], now.year());
                let wish = if bday { format!(" Et c'est votre anniversaire aujourd'hui{}", name.map(|n| format!(", joyeux anniversaire {n} !")).unwrap_or_else(|| " ! Joyeux anniversaire !".into())) } else { String::new() };
                json!({ "say": format!("Nous sommes le {formatted}.{wish}"), "display": formatted })
            }));
        }
        // 3. Répétition
        if crate::re!(r"\b(repete|tu as dit quoi|qu est ce que tu as dit|qu as tu dit|redis( moi)?)\b").is_match(text) {
            return Some(it(o, "repeat", "Répétition", "dernière phrase", |o| async move {
                let last = o.last_spoken.borrow().clone();
                if last.is_empty() {
                    return json!({ "say": "Je n'ai encore rien dit.", "display": "Rien à répéter" });
                }
                json!({ "say": format!("J'ai dit : {last}"), "display": last })
            }));
        }
        // 4. Volume
        if crate::re!(r"\b(monte|augmente)\s+(le son|le volume|la musique)\b").is_match(text) || crate::re!(r"\bplus fort\b").is_match(text) {
            return Some(it(o, "volume_up", "Volume +", "amplification sonore", |_| async move {
                let _ = super::system::volume(&json!({ "action": "up" })).await;
                json!({ "say": "Volume augmenté.", "display": "Volume +" })
            }));
        }
        if crate::re!(r"\b(baisse|diminue)\s+(le son|le volume|la musique)\b").is_match(text) || crate::re!(r"\bmoins fort\b").is_match(text) {
            return Some(it(o, "volume_down", "Volume -", "atténuation sonore", |_| async move {
                let _ = super::system::volume(&json!({ "action": "down" })).await;
                json!({ "say": "Volume diminué.", "display": "Volume -" })
            }));
        }
        if crate::re!(r"\b(coupe|remets|mute)\s+(le son|le volume)\b").is_match(text) || crate::re!(r"\bsilence\b").is_match(text) {
            return Some(it(o, "volume_mute", "Mute son", "bascule muet", |_| async move {
                let _ = super::system::volume(&json!({ "action": "mute" })).await;
                json!({ "say": "Son basculé.", "display": "Mute" })
            }));
        }
        if let Some(m) = crate::re!(r"\b(?:mets?|regle)\s+(?:le\s+son|le\s+volume)\s+(?:a|a\s+environ)\s+(\d{1,3})\s*(?:%|pour\s*cent)?\b").captures(text) {
            let v = m[1].parse::<i64>().unwrap_or(0).clamp(0, 100);
            return Some(it(o, "volume_set", &format!("Volume {v}%"), &format!("réglage à {v}%"), move |_| async move {
                let _ = super::system::volume(&json!({ "niveau": v })).await;
                json!({ "say": format!("Volume réglé à {v} pour cent."), "display": format!("{v}%") })
            }));
        }
        // 5. Minuteur
        if crate::re!(r"\b(annule|arrete|supprime|stop)\s+(le\s+)?minuteur\b").is_match(text) {
            return Some(it(o, "timer_cancel", "Arrêt minuteur", "annulation du décompte", |o| async move {
                let n = {
                    let mut t = o.router.timers.borrow_mut();
                    let n = t.len();
                    for (_, tm) in t.drain() {
                        tm.clear();
                    }
                    n
                };
                json!({ "say": if n > 0 { "Minuteur annulé." } else { "Aucun minuteur n'était actif." }, "display": "Minuteur annulé" })
            }));
        }
        if let Some(m) = crate::re!(r"\b(?:minuteur|mets\s+un\s+minuteur|alarme|compte\s+a\s+rebours|rappelle\s*moi)\s+(?:de\s+|dans\s+)?(\d+)\s*(seconde|secondes|sec|s|minute|minutes|min|m|heure|heures|h)?\b").captures(text) {
            let val: i64 = m[1].parse().unwrap_or(0);
            let unit = m.get(2).map(|x| x.as_str().to_lowercase()).unwrap_or_else(|| "minute".into());
            let (ms, mut unit_label) = if unit.starts_with('s') {
                (val * 1000, "seconde".to_string())
            } else if unit.starts_with('h') {
                (val * 3_600_000, "heure".to_string())
            } else {
                (val * 60000, "minute".to_string())
            };
            if val > 1 {
                unit_label.push('s');
            }
            let ul = unit_label.clone();
            return Some(it(o, "timer_create", &format!("Minuteur {val} {unit_label}"), &format!("décompte {val} {unit_label}"), move |o| {
                let ul = ul.clone();
                async move {
                    let r = &o.router;
                    r.timer_counter.set(r.timer_counter.get() + 1);
                    let id = format!("timer_{}", r.timer_counter.get());
                    let timer = Timer::new();
                    let (oc, id2, ul2) = (r.o.clone(), id.clone(), ul.clone());
                    timer.set(ms.max(0) as u64, move || {
                        let orch = oc.up();
                        orch.router.timers.borrow_mut().remove(&id2);
                        orch.emit(json!({ "type": "say", "text": format!("Grelot ! Le minuteur de {val} {ul2} est terminé.") }));
                        let alert = format!("alarm_{}", js::now_ms());
                        orch.emit(json!({ "type": "create", "id": alert, "label": "Minuteur terminé !", "detail": format!("{val} {ul2} écoulées") }));
                        let oc2 = oc.clone();
                        js::set_timeout(6000, move || oc2.up().emit(json!({ "type": "finish", "id": alert, "status": "done" })));
                    });
                    r.timers.borrow_mut().insert(id, timer);
                    json!({ "say": format!("C'est parti pour {val} {ul}. Je te préviens dès que c'est fini."), "display": format!("Minuteur {val} {ul}") })
                }
            }));
        }
        // 6. Application connue
        if let Some(m) = crate::re!(r"\b(?:ouvre|lance|demarre)\s+([a-z0-9._-]+)\b").captures(text) {
            let c = m[1].trim().to_string();
            if let Some(exe) = super::system::app_registry(&c) {
                let c2 = c.clone();
                let mut i = it(o, "open_app", &format!("Lancement {c}"), &format!("exécutable {exe}"), move |_| {
                    let c = c2.clone();
                    async move {
                        let _ = super::system::ouvrir_application(&json!({ "nom": c })).await;
                        json!({ "say": format!("J'ouvre {c}."), "display": c })
                    }
                });
                i.detail = format!("exécutable {exe}");
                return Some(i);
            }
        }
        // 7. Mémoire
        if crate::re!(r"\b(qui suis je|que sais tu de moi|mes informations|mes souvenirs|ce que tu sais sur moi)\b").is_match(text) && !crate::re!(r"\b(?:oublie|efface|supprime)\b").is_match(text) {
            return Some(it(o, "memory_info", "Mémoire utilisateur", "souvenirs enregistrés", |o| async move {
                let entries = o.memory.list();
                if entries.is_empty() {
                    return json!({ "say": "Je n'ai pas encore d'informations durables enregistrées sur toi.", "display": "Mémoire vide" });
                }
                let intro = o.memory.get_user_name().map(|n| format!("Tu t'appelles {n}. ")).unwrap_or_default();
                let summary = entries.iter().map(|e| js::gs(e, "text")).collect::<Vec<_>>().join(". ");
                json!({ "say": format!("{intro}Voici ce que je sais de toi : {summary}"), "display": format!("{} souvenirs", entries.len()) })
            }));
        }
        // 8. Capture d'écran
        if crate::re!(r"\b(prends? (une )?capture d ecran|fais (une )?capture|capture d ecran|screenshot)\b").is_match(text) {
            return Some(it(o, "screenshot", "Capture d'écran", "sauvegarde PNG", |_| async move {
                let _ = super::system::capture_ecran(&json!({})).await;
                json!({ "say": "Capture d'écran effectuée et sauvegardée.", "display": "Capture sauvegardée" })
            }));
        }
        // 8a-0. Bureau JARVIS
        let desk_off = crate::re!(r"\b(?:desactive|arrete|coupe|enleve|supprime) (?:le )?bureau jarvis\b").is_match(text);
        let desk_on = !desk_off && crate::re!(r"\b(?:active|allume|installe|mets en place) (?:le )?bureau jarvis\b").is_match(text);
        let desk_go = !desk_off && !desk_on && crate::re!(r"\b(?:va|vas|aller|passe|bascule|emmene moi|amene moi|montre moi|affiche|ouvre)(?: sur| au| vers| le| moi le)* bureau jarvis\b").is_match(text);
        if desk_on || desk_off || desk_go {
            let kind = if desk_go { "desk_goto" } else if desk_on { "desk_on" } else { "desk_off" };
            return Some(it(o, kind, "Bureau JARVIS", if desk_go { "bureau virtuel" } else if desk_on { "activation" } else { "désactivation" }, move |o| async move {
                let enabled = js::gb(&o.config.get("desk"), "enabled");
                if desk_go && enabled {
                    o.emit(json!({ "type": "desk_goto" }));
                    return json!({ "say": "Bureau JARVIS.", "display": "Bureau JARVIS" });
                }
                let on = !desk_off;
                if on != enabled {
                    o.emit(json!({ "type": "desk_enable", "on": on }));
                }
                json!({
                    "say": if on { if enabled { "Le bureau JARVIS est déjà actif." } else { "J’installe le bureau JARVIS sur le bureau virtuel deux." } } else { "Bureau JARVIS désactivé." },
                    "display": if on { "Bureau JARVIS actif" } else { "Bureau JARVIS désactivé" }
                })
            }));
        }
        // 8a. Mode widget / plein écran
        let w_on = crate::re!(r"\b(?:mode widget|mini widget|widget flottant|mini orbe|reduis jarvis|passe en widget)\b").is_match(text);
        let w_off = crate::re!(r"\b(?:plein ecran|agrandis jarvis|remets jarvis en grand|quitte le mode widget|sors du mode widget|interface complete)\b").is_match(text);
        if w_on || w_off {
            let on = w_on && !w_off;
            return Some(it(o, if on { "widget_on" } else { "widget_off" }, if on { "Mode widget" } else { "Plein écran" }, if on { "orbe flottante" } else { "interface complète" }, move |o| async move {
                o.emit(json!({ "type": "widget_mode", "on": on }));
                json!({ "say": if on { "Je passe en mode widget." } else { "Interface complète." }, "display": if on { "Mode widget" } else { "Plein écran" } })
            }));
        }
        // 8a bis. Vocabulaire
        if let Some(v) = self.vocabulary_intent(raw, text) {
            return Some(v);
        }
        // 8b. Lecture d'écran par OCR
        if crate::re!(r"^(?:/ocr|/ecran|/screen|/lire_ecran)$").is_match(text)
            || crate::re!(r"\b(?:lis moi cette erreur|lis le message d erreur|lis l erreur|quel est le message d erreur|lis l erreur a l ecran|lis l erreur sur l ecran)\b").is_match(text)
            || crate::re!(r"\b(?:lis ce qui est affiche|lis le texte a l ecran|lis l ecran|qu est ce qui est ecrit a l ecran|qu est ce qui est ecrit sur l ecran)\b").is_match(text)
        {
            let err_only = crate::re!(r"\b(?:erreur|error|exception|message d erreur)\b").is_match(text);
            return Some(it(o, "read_screen_ocr", "Vision de l'écran (OCR)", if err_only { "lecture du message d'erreur" } else { "lecture du texte affiché" }, move |o| async move {
                let res = super::system::lire_ecran_ocr(&json!({ "filtrer_erreur": err_only })).await.unwrap_or(Value::Null);
                if js::truthy(&res["texte"]) {
                    o.record("lecture_ecran_ocr", json!({ "chemin": res["chemin"], "resume": res["resume"] }));
                    let resume = js::gs(&res, "resume");
                    let prefix = if err_only { "Je lis sur ton écran : " } else { "Voici le texte détecté à l'écran : " };
                    return json!({ "say": format!("{prefix}{resume}"), "display": js::slice_chars(&resume, 40) });
                }
                json!({ "say": "Aucun texte ou message d'erreur lisible n'a été détecté sur l'écran.", "display": "Aucun texte détecté" })
            }));
        }
        // 9. Autorisation temporaire d'écriture dans une zone
        if let Some(m) = crate::re!(r"\b(?:je travaille sur|autorise l ecriture( dans)?|autorise)\s*(?:ce projet|la zone|le dossier)?\s*([a-z0-9._-]+)?\s*(?:pendant|pour)\s*(\d+|un|une|deux|trois|quatre)\s*(h|heure|heures|min|minute|minutes)?\b").captures(text) {
            let zone = m.get(2).map(|x| x.as_str().to_string()).unwrap_or_else(|| "projects".into());
            let amount: f64 = match &m[3] {
                "un" | "une" => 1.0,
                "deux" => 2.0,
                "trois" => 3.0,
                "quatre" => 4.0,
                n => n.parse::<f64>().ok().filter(|x| *x != 0.0).unwrap_or(1.0),
            };
            let unit = m.get(4).map(|x| x.as_str().to_string()).unwrap_or_else(|| "h".into());
            let minutes = if unit.starts_with('m') { amount } else { amount * 60.0 };
            return Some(it(o, "temporary_authorization", "Autorisation temporaire", &format!("{} min d'écriture libre", js::num_str(minutes)), move |o| {
                let zone = zone.clone();
                async move {
                    let Some(auth) = o.zones.grant_temporary(&zone, minutes) else { return json!({ "say": "Impossible d'identifier la zone pour l'autorisation.", "display": "Zone introuvable" }) };
                    o.record("autorisation_temporaire", json!({ "zone": auth["zone"]["name"], "chemin": auth["zone"]["path"], "duree_minutes": auth["durationMinutes"], "expire_a": auth["expiresAt"] }));
                    let (name, dur) = (js::gs(&auth["zone"], "name"), js::s(&auth["durationMinutes"]));
                    json!({ "say": format!("Écriture autorisée sans confirmation dans la zone {name} pendant {dur} minutes."), "display": format!("{name} ({dur}m)") })
                }
            }));
        }
        // 10. Sourdine contextuelle
        if crate::re!(r"\b(?:mode\s+)?(?:discret|silencieux|passe\s+en\s+silence)\b").is_match(text)
            || crate::re!(r"\b(?:sourdine\s+automatique|sourdine\s+auto)\b").is_match(text)
            || crate::re!(r"\b(?:active|mets)\s+(?:la\s+)?(?:sourdine|le\s+silence)\b").is_match(text)
            || crate::re!(r"\b(?:desactive|coupe|arrete|arrête)\s+(?:la\s+)?(?:sourdine|le\s+mode\s+discret)\b").is_match(text)
            || crate::re!(r"\b(?:remets|reactive|réactive)\s+(?:la\s+)?voix\b").is_match(text)
            || crate::re!(r"^/(?:sourdine|mute|discret|silence)").is_match(text)
        {
            let mode = if crate::re!(r"\b(?:mode\s+)?(?:discret|silencieux|silence)\b").is_match(text) || crate::re!(r"\b(?:active|mets)\s+(?:la\s+)?(?:sourdine)\b").is_match(text) || crate::re!(r"^/(?:sourdine|mute)\s+on\b").is_match(text) {
                "on"
            } else if crate::re!(r"\b(?:desactive|coupe|remets|reactive|réactive)\b").is_match(text) || crate::re!(r"^/(?:sourdine|mute)\s+off\b").is_match(text) {
                "off"
            } else {
                "auto"
            };
            return Some(it(o, "context_mute", "Sourdine contextuelle", &format!("Mode {mode}"), move |o| async move {
                let state = match o.context_mute.set_mode(mode).await {
                    Ok(s) => s,
                    Err(_) => return json!({ "say": "Gestionnaire de sourdine non disponible.", "display": "Erreur" }),
                };
                o.record("changement_sourdine", state.clone());
                o.publish_status();
                let label = match js::gs(&state, "mode").as_str() {
                    "auto" => "automatique (silence lors des jeux, visios et enregistrements)",
                    "on" => "activée (silence complet)",
                    _ => "désactivée (voix active)",
                };
                json!({ "say": format!("Sourdine contextuelle réglée sur {label}."), "display": format!("Sourdine : {mode}") })
            }));
        }
        // 11. Thèmes visuels
        if crate::re!(r"\b(?:theme|thème)\s+(?:cyan|bleu|dore|doré|or|ambre|gold|vert|matrix|emeraude|émeraude|rouge|crimson|rubis|violet|synthwave|amethyste|améthyste|pourpre|d['’ ]?origine|standard|hologramme)\b").is_match(text) || crate::re!(r"(?i)^/(?:theme|thème|style)\s+([a-z]+)").is_match(text) {
            let t = if crate::re!(r"\b(dore|doré|or|ambre|gold)\b").is_match(text) {
                "gold"
            } else if crate::re!(r"\b(vert|matrix|emeraude|émeraude)\b").is_match(text) {
                "matrix"
            } else if crate::re!(r"\b(rouge|crimson|rubis)\b").is_match(text) {
                "crimson"
            } else if crate::re!(r"\b(violet|synthwave|amethyste|améthyste|pourpre)\b").is_match(text) {
                "violet"
            } else {
                "cyan"
            };
            let label = match t {
                "gold" => "Stark Ambre & Or",
                "matrix" => "Émeraude Matrix",
                "crimson" => "Rubis Crimson",
                "violet" => "Synthwave Améthyste",
                _ => "Cyan Hologramme",
            };
            return Some(it(o, "theme", &format!("Thème {label}"), "interface holographique", move |o| async move {
                o.status.d.borrow_mut().theme = t.into();
                o.status.save();
                o.record("changement_theme", json!({ "theme": t }));
                o.emit(json!({ "type": "theme_config", "theme": t }));
                o.publish_status();
                json!({ "say": format!("Thème visuel changé pour {label}."), "display": format!("Thème : {label}") })
            }));
        }
        // 12. Densité d'affichage
        if crate::re!(r"\b(?:mode\s+compact|affichage\s+compact|densite\s+compacte)\b").is_match(text)
            || crate::re!(r"\b(?:mode\s+large|affichage\s+grand|grand\s+affichage|densite\s+large)\b").is_match(text)
            || crate::re!(r"\b(?:mode\s+normal|affichage\s+normal|densite\s+normale)\b").is_match(text)
            || crate::re!(r"(?i)^/(?:densite|densité|density)\s+([a-z]+)").is_match(text)
        {
            let d = if crate::re!(r"\bcompact\b").is_match(text) {
                "compact"
            } else if crate::re!(r"\b(large|grand)\b").is_match(text) {
                "large"
            } else {
                "normal"
            };
            let label = match d {
                "compact" => "compacte (panneaux denses)",
                "large" => "large (haute lisibilité)",
                _ => "normale (équilibrée)",
            };
            return Some(it(o, "density", &format!("Densité {d}"), label, move |o| async move {
                o.status.d.borrow_mut().density = d.into();
                o.status.save();
                o.record("changement_densite", json!({ "densite": d }));
                o.emit(json!({ "type": "density_config", "density": d }));
                o.publish_status();
                json!({ "say": format!("Densité d'affichage réglée sur {label}."), "display": format!("Densité : {d}") })
            }));
        }
        // 14. Empreinte vocale et reconnaissance du locuteur
        let enroll = crate::re!(r"(?i)\b(?:enregistre|apprends|m[eé]morise|ajoute)\s+(?:la\s+voix\s+de\s+|l\s+empreinte\s+de\s+)([a-zA-ZÀ-ÖØ-öø-ÿ'-]+)").captures(raw).or_else(|| crate::re!(r"(?i)^/(?:locuteur|speaker)\s+(?:apprendre|enregistrer|enrol|ajouter|add)\s+([a-zA-ZÀ-ÖØ-öø-ÿ'-]+)").captures(raw));
        let enroll_self = crate::re!(r"\b(?:enregistre|apprends|m[eé]morise)\s+ma\s+voix\b").is_match(text) || crate::re!(r"(?i)^/(?:locuteur|speaker)\s+(?:apprendre|enregistrer|enrol)\b").is_match(text);
        if enroll.is_some() || enroll_self {
            let name = match &enroll {
                Some(m) => {
                    let n = m[1].trim().to_lowercase();
                    js::capitalize(&n)
                }
                None => {
                    let orch = o.up();
                    let cs = orch.current_speaker.borrow().clone();
                    cs.or_else(|| orch.memory.get_user_name()).unwrap_or_else(|| "Johann".into())
                }
            };
            let n2 = name.clone();
            return Some(it(o, "speaker_enroll", &format!("Apprentissage vocal ({name})"), &format!("Empreinte de {name}"), move |o| {
                let name = n2.clone();
                async move {
                    o.record("enrolement_locuteur_demarre", json!({ "nom": name, "echantillons": 8, "origine": "vocal" }));
                    o.emit(json!({ "type": "speaker_enroll_start", "name": name, "count": 8 }));
                    json!({ "say": format!("C'est parti pour l'enregistrement de {name}. Prononce huit phrases variées, une à une, pour que j'apprenne cette empreinte vocale."), "display": format!("Enrôlement : {name}") })
                }
            }));
        }
        let delete = crate::re!(r"(?i)\b(?:supprime|efface|retire|oublie)\s+(?:la\s+voix\s+de\s+|l\s+empreinte\s+de\s+)([a-zA-ZÀ-ÖØ-öø-ÿ'-]+)").captures(raw).or_else(|| crate::re!(r"(?i)^/(?:locuteur|speaker)\s+(?:supprimer|delete|remove|retirer)\s+([a-zA-ZÀ-ÖØ-öø-ÿ'-]+)").captures(raw));
        if let Some(m) = delete {
            let name = m[1].trim().to_string();
            return Some(it(o, "speaker_delete", &format!("Suppression profil vocal ({name})"), "Suppression empreinte", move |o| {
                let name = name.clone();
                async move {
                    let st = o.get_speaker_status();
                    let lower = name.to_lowercase();
                    let Some(t) = js::arr(&st["speakers"]).into_iter().find(|s| js::gs(s, "id") == lower || js::gs(s, "name").to_lowercase() == lower) else {
                        return json!({ "say": format!("Je ne trouve pas de profil vocal au nom de {name}."), "display": format!("Profil introuvable : {name}") });
                    };
                    o.record("suppression_locuteur", json!({ "id": t["id"], "nom": t["name"], "origine": "vocal" }));
                    o.emit(json!({ "type": "speaker_delete", "id": t["id"] }));
                    let n = js::gs(&t, "name");
                    json!({ "say": format!("Le profil vocal de {n} a été supprimé."), "display": format!("Profil {n} supprimé") })
                }
            }));
        }
        if crate::re!(r"(?i)\b(?:liste\s+(?:les\s+)?voix|qui\s+(?:a\s+son\s+empreinte|est\s+enregistr[eé])|quelles\s+sont\s+les\s+voix\s+enregistr[eé]es|liste\s+(?:les\s+)?locuteurs)\b").is_match(raw) || crate::re!(r"(?i)^/(?:locuteur|speaker)\s+(?:liste|list)\b").is_match(text) {
            return Some(it(o, "speaker_list", "Liste des profils vocaux", "Empreintes enregistrées", |o| async move {
                let st = o.get_speaker_status();
                let speakers = js::arr(&st["speakers"]);
                let count = st["speakersCount"].as_u64().map(|c| c as usize).filter(|c| *c != 0).unwrap_or(speakers.len());
                o.list_cards.voices_card();
                if count == 0 {
                    return json!({ "say": "Aucune empreinte vocale n'est enregistrée pour le moment.", "display": "0 empreinte" });
                }
                let names = speakers.iter().map(|s| format!("{}{}", js::gs(s, "name"), if js::gb(s, "isPrimary") { " (principal)" } else { "" })).collect::<Vec<_>>().join(", ");
                let p = if count > 1 { "s" } else { "" };
                json!({ "say": format!("Il y a {count} profil{p} vocal enregistré{p} : {names}."), "display": format!("{count} profil(s) : {names}") })
            }));
        }
        if crate::re!(r"\b(?:ne\s+r[eé]ponds?\s+qu['’ ]+[aà]\s+ma\s+voix|active\s+(?:la\s+)?reconnaissance\s+(?:du\s+)?locuteur)\b").is_match(text) || crate::re!(r"(?i)^/(?:locuteur|speaker)\s+(?:on|activer)\b").is_match(text) {
            return Some(it(o, "speaker_mode", "Filtrage locuteur", "Activé", |o| async move {
                o.record("changement_mode_locuteur", json!({ "actif": true }));
                o.emit(json!({ "type": "speaker_set_enabled", "enabled": true }));
                o.publish_status();
                json!({ "say": "Filtrage par empreinte vocale activé. Je ne répondrai désormais qu'aux voix reconnues.", "display": "Locuteur actif" })
            }));
        }
        if crate::re!(r"\b(?:d[eé]sactive\s+(?:la\s+)?reconnaissance\s+(?:du\s+)?locuteur)\b").is_match(text) || crate::re!(r"\b(?:accepte\s+toutes\s+les\s+voix)\b").is_match(text) || crate::re!(r"(?i)^/(?:locuteur|speaker)\s+(?:off|desactiver)\b").is_match(text) {
            return Some(it(o, "speaker_mode", "Filtrage locuteur", "Désactivé", |o| async move {
                o.record("changement_mode_locuteur", json!({ "actif": false }));
                o.emit(json!({ "type": "speaker_set_enabled", "enabled": false }));
                o.publish_status();
                json!({ "say": "Filtrage de locuteur désactivé. Toutes les voix sont acceptées.", "display": "Toutes voix" })
            }));
        }
        // 13. Salutation seule
        if crate::re!(r"^(?:bonjour|salut|bonsoir|coucou|hello|hey|yo)(?:\s+jarvis)?(?:\s+(?:comment (?:ca va|vas tu|allez vous)|ca va))?[.\s]*$").is_match(text) {
            return Some(it(o, "greeting", "Salutation", "règles de politesse", |o| async move {
                let h = Local::now().hour();
                let salutation = if h >= 18 || h < 5 { "Bonsoir" } else { "Bonjour" };
                let bday = o.memory.is_birthday_today();
                let a = o.addressing_forms();
                let current = o.current_speaker.borrow().clone();
                let user = current.clone().or_else(|| Some(a.name.clone()).filter(|n| !n.is_empty()));
                let appellation = match &current {
                    Some(c) => format!(" {c}"),
                    None => if a.call.is_empty() { String::new() } else { format!(" {}", a.call) },
                };
                let wish = if bday { " Et un très joyeux anniversaire !" } else { "" };
                let replies = [
                    format!("{salutation}{appellation}.{wish} Je suis à {} entière disposition. Que puis-je faire pour {} ?", a.ton, a.toi),
                    format!("{salutation}{}.{wish} Ravi de {}. Comment puis-je {} aujourd'hui ?", user.as_ref().map(|u| format!(" {u}")).unwrap_or_default(), a.cod("retrouver"), a.cod("aider")),
                    format!("{salutation}{appellation}.{wish} Tous les voyants sont au vert. Que {} accomplir ?", if a.vous { "souhaitez-vous" } else { "souhaites-tu" }),
                ];
                let reply = replies[js::rand_below(replies.len())].clone();
                json!({ "say": reply, "display": format!("{salutation}{} !", user.map(|u| format!(" {u}")).unwrap_or_default()) })
            }));
        }
        // 14 a. Checkup : le cœur passé au scanner (card Checkup), fonctions vitales de Jarvis.
        if crate::re!(r"(?:check[ -]?up|scanne[- ]toi|fais toi scanner|etat de sante|bilan de sante|tes fonctions vitales|scan (?:complet|du coeur))").is_match(text) || crate::re!(r"(?i)^/checkup").is_match(text) {
            return Some(it(o, "checkup", "Checkup", "scan des fonctions vitales", |o| async move {
                o.show_checkup_card();
                let r = o.checkup_report();
                json!({ "say": Orchestrator::checkup_summary(&r), "display": format!("Checkup : {} %", js::s(&r["score"])) })
            }));
        }
        // 14. Auto-diagnostic
        if crate::re!(r"\b(?:diagnostic|check\s+systeme|verifie\s+les\s+systemes|etat\s+des\s+systemes|tout\s+va\s+bien|test\s+systeme|statut\s+systeme|auto[- ]diagnostic|check)\b").is_match(text) || crate::re!(r"(?i)^/(?:check|diagnostic|diag)").is_match(text) {
            return Some(it(o, "self_check", "Diagnostic système", "contrôle d’intégrité et de santé", |o| async move {
                let r = o.perform_self_check(true).await;
                o.show_checkup_card();
                json!({
                    "say": r["spoken"],
                    "display": if js::gb(&r, "ok") { "Diagnostic : Nominal (100%)".to_string() } else { format!("Attention : {}", js::str_list(&r["issues"]).join(", ")) }
                })
            }));
        }
        // 15. Journal des erreurs
        if crate::re!(r"\b(?:erreurs?|quelles\s+sont\s+les\s+erreurs|affiche\s+les\s+erreurs|liste\s+les\s+erreurs|voir\s+les\s+logs|affiche\s+les\s+logs|journal\s+des\s+erreurs|y\s+a\s+t\s+il\s+des\s+erreurs)\b").is_match(text) || crate::re!(r"(?i)^/(?:erreurs?|errors?|logs?)").is_match(text) {
            return Some(it(o, "error_logs", "Journal des erreurs", "audit et historique d’incidents", |_| async move {
                let logger = crate::logger::logger();
                let errors = logger.recent_errors(5);
                if errors.is_empty() {
                    return json!({ "say": "Aucune erreur n'est enregistrée dans le journal. Tous les composants fonctionnent nominalement.", "display": "Erreurs : 0 (système sain)" });
                }
                json!({ "say": logger.summary_sentence(), "display": format!("{} incident(s) : dernier sur {}", errors.len(), js::gs(&errors[0], "source")) })
            }));
        }
        // 16. Télémétrie
        if crate::re!(r"\b(?:cpu|ram|memoire\s+vive|charge\s+systeme|utilisation\s+cpu|utilisation\s+ram|performances?|télémétrie|telemetrie)\b").is_match(text) || crate::re!(r"(?i)^/(?:cpu|ram|telemetrie|perf)").is_match(text) {
            return Some(it(o, "telemetry", "Télémétrie Système", "charge CPU & RAM", |_| async move {
                let t = super::system::telemetry();
                let (cpu, pct, used, total) = (js::s(&t["cpu"]), js::s(&t["ramPct"]), js::s(&t["ramUsedGb"]), js::s(&t["ramTotalGb"]));
                json!({
                    "say": format!("Le processeur est utilisé à {cpu}%. La mémoire vive est occupée à {pct}%, soit {used} Go sur {total} Go au total."),
                    "display": format!("CPU: {cpu}% · RAM: {used}/{total} Go ({pct}%)")
                })
            }));
        }
        // 17. Périphérique audio
        if crate::re!(r"\b(?:bascule|passe|mets|active|change)\s+(?:sur\s+|pour\s+)?(?:le\s+casque|les\s+enceintes|le\s+pc|l['’ ]?ordinateur|les\s+haut[- ]parleurs)\b").is_match(text)
            || crate::re!(r"\b(?:bascule\s+l['’ ]?audio|change\s+de\s+sortie|sortie\s+audio|peripherique\s+audio)\b").is_match(text)
            || crate::re!(r"(?i)^/(?:audio|casque|enceinte|enceintes|speakers|headset)").is_match(text)
        {
            let to_pc = crate::re!(r"\b(?:enceinte|enceintes|pc|ordinateur|haut[- ]parleur|haut[- ]parleurs)\b").is_match(text) || crate::re!(r"(?i)^/(?:enceinte|enceintes|speakers|pc)").is_match(text);
            let to_headset = crate::re!(r"\b(?:casque|headset|ecouteur|ecouteurs)\b").is_match(text) || crate::re!(r"(?i)^/(?:casque|headset)").is_match(text);
            let detail = if to_headset { "vers Casque" } else if to_pc { "vers Enceintes PC" } else { "bascule audio" };
            return Some(it(o, "audio_toggle", "Périphérique Audio", detail, move |o| async move {
                let res = if to_headset {
                    super::system::set_audio_device("casque").await
                } else if to_pc {
                    super::system::set_audio_device("pc").await
                } else {
                    super::system::toggle_audio_device().await
                };
                let mut ev = json!({ "type": "audio_device" });
                js::assign(&mut ev, res.clone());
                o.emit(ev);
                let headset = js::gs(&res, "mode") == "casque";
                json!({
                    "say": format!("Périphérique audio basculé {}.", if headset { "sur le casque" } else { "sur les enceintes et le micro de l’ordinateur" }),
                    "display": if headset { "Audio : Casque" } else { "Audio : Enceintes PC" }
                })
            }));
        }
        // 18. Purge de la mémoire
        if crate::re!(r"(?i)^/(?:memoire\s+(?:reset|effacer|vider|purge)|reset[-_]memoire|effacer[-_]memoire|purge[-_]memoire|rag[-_]reset|reset[-_]rag)$").is_match(text)
            || crate::re!(r"\b(?:efface|reinitialise|réinitialise|vide|purge)\s+(?:toute\s+)?(?:la|ta|ton|le|tes|les)\s+(?:memoire|mémoire|rag|souvenirs)\b").is_match(text)
            || crate::re!(r"\b(?:oublie\s+tout\s+(?:ce\s+que\s+tu\s+sais|sur\s+moi))\b").is_match(text)
            || crate::re!(r"\brepars?\s+(?:a\s+zero|de\s+zero|vierge)\b").is_match(text)
        {
            return Some(it(o, "memory_clear", "Réinitialisation Mémoire", "purge intégrale RAG", |o| async move {
                // Action destructrice : validation selon la section Sécurité ; la confirmation suit l'effacement.
                o.request_memory_purge();
                json!({ "say": null, "display": "Effacement de la mémoire" })
            }));
        }
        // 19. Faire connaissance
        if crate::re!(r"(?i)^/(?:onboarding|faire[-_ ]connaissance|presentation)$").is_match(text) || crate::re!(r"\b(?:faisons\s+connaissance|apprends\s+qui\s+je\s+suis|pose\s+moi\s+les\s+questions)\b").is_match(text) {
            return Some(it(o, "onboarding_start", "Faisons connaissance", "questions de présentation", |o| async move {
                o.start_onboarding();
                json!({ "say": "", "display": "Onboarding actif" })
            }));
        }
        None
    }

    /// « apprends le mot Worganic », « /vocabulaire corrige worganique = Worganic », « retire Shelley du vocabulaire ».
    fn vocabulary_intent(&self, raw: &str, text: &str) -> Option<Intent> {
        let raw = crate::re!(r"[.!?]+$").replace(raw.trim(), "").into_owned();
        let o = &self.o;
        let done = |kind: &str, label: &str, detail: &str, run: Rc<dyn Fn(&Orchestrator) -> J>| {
            it(o, kind, label, detail, move |o| {
                let run = run.clone();
                async move {
                    let r = run(&o);
                    o.publish_vocabulary(true);
                    r
                }
            })
        };
        let correct = crate::re!(r"(?i)^(?:/vocabulaire\s+corrige|/corrige)\s+(.+?)\s*(?:=>|->|→|=)\s*(.+)$").captures(&raw)
            .or_else(|| crate::re!(r"(?i)^quand (?:tu entends|je dis|je prononce)\s+(.+?),?\s+(?:c['’]est|comprends|écris|ecris|ça veut dire)\s+(.+)$").captures(&raw))
            .or_else(|| crate::re!(r"(?i)^associe\s+(?:la prononciation\s+)?(.+?)\s+(?:à|a|au mot)\s+(.+)$").captures(&raw));
        if let Some(m) = correct {
            let (heard, right) = (m[1].trim().to_string(), m[2].trim().to_string());
            let (h2, r2) = (heard.clone(), right.clone());
            return Some(done("vocabulary_correct", "Vocabulaire", &format!("{heard} → {right}"), Rc::new(move |o| {
                let pair = o.vocabulary.correct(&h2, &r2);
                o.record("vocabulaire_correction", json!({ "entendu": h2, "correct": r2 }));
                if pair.is_some() { json!({ "say": format!("D'accord, quand j'entendrai {h2}, j'écrirai {r2}."), "display": format!("{h2} → {r2}") }) } else { json!({ "say": "Cette correction ne change rien.", "display": "Correction ignorée" }) }
            })));
        }
        let add = crate::re!(r"(?i)^/vocabulaire\s+(?:ajoute|ajouter|add)\s+(.+)$").captures(&raw)
            .or_else(|| crate::re!(r"(?i)\b(?:ajoute|apprends)\s+(?:le\s+(?:mot|terme|nom)\s+)?(.+?)\s+(?:au|à\s+ton|a\s+ton|dans\s+(?:le|ton))\s+vocabulaire$").captures(&raw))
            .or_else(|| crate::re!(r"(?i)\bapprends\s+le\s+(?:mot|terme|nom)\s+(.+)$").captures(&raw));
        if let Some(m) = add {
            let term = m[1].trim().to_string();
            let t2 = term.clone();
            return Some(done("vocabulary_add", "Vocabulaire", &format!("+ {term}"), Rc::new(move |o| match o.vocabulary.add(&t2) {
                Some(added) => {
                    o.record("vocabulaire_ajout", json!({ "terme": added }));
                    json!({ "say": format!("J'ai ajouté {added} à mon vocabulaire."), "display": format!("+ {added}") })
                }
                None => json!({ "say": "Je n'ai pas compris quel mot ajouter.", "display": "Mot invalide" }),
            })));
        }
        let remove = crate::re!(r"(?i)^/vocabulaire\s+(?:retire|supprime|enl[eè]ve|remove)\s+(.+)$").captures(&raw)
            .or_else(|| crate::re!(r"(?i)\b(?:retire|supprime|enl[eè]ve|oublie)\s+(?:le\s+(?:mot|terme|nom)\s+)?(.+?)\s+du\s+vocabulaire$").captures(&raw));
        if let Some(m) = remove {
            let term = m[1].trim().to_string();
            let t2 = term.clone();
            return Some(done("vocabulary_remove", "Vocabulaire", &format!("− {term}"), Rc::new(move |o| {
                if o.vocabulary.remove(&t2) {
                    o.record("vocabulaire_retrait", json!({ "terme": t2 }));
                    json!({ "say": format!("{t2} est retiré de mon vocabulaire."), "display": format!("− {t2}") })
                } else {
                    json!({ "say": format!("{t2} ne fait pas partie des mots que tu m'as appris."), "display": "Mot absent" })
                }
            })));
        }
        if text == "/vocabulaire" || crate::re!(r"\b(?:quel est ton vocabulaire|liste (?:le|ton) vocabulaire|quels mots (?:connais tu|as tu appris))\b").is_match(text) {
            return Some(done("vocabulary_list", "Vocabulaire", "mots reconnus", Rc::new(|o| {
                let b = o.vocabulary.build();
                let terms = js::arr(&b["terms"]).len();
                let corrections = js::arr(&b["corrections"]).len();
                let learned = o.vocabulary.terms.borrow().clone();
                let say = if learned.is_empty() {
                    format!("Tu ne m'as encore appris aucun mot. J'utilise {terms} termes, dont les noms tirés de ta mémoire.")
                } else {
                    format!("Tu m'as appris {} mot{} : {}. J'utilise {terms} termes au total, avec les noms tirés de ta mémoire.", learned.len(), if learned.len() > 1 { "s" } else { "" }, learned.iter().take(12).cloned().collect::<Vec<_>>().join(", "))
                };
                json!({
                    "say": if corrections > 0 { format!("{say} Et {corrections} correction{}.", if corrections > 1 { "s" } else { "" }) } else { say },
                    "display": format!("{terms} termes")
                })
            })));
        }
        None
    }

    pub fn log(&self, m: String) {
        (self.log)(m)
    }
}
