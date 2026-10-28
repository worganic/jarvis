// Annonces spontanées encadrées (idée reprise de l'« interpellation » du projet Cornelius) : tout ce que Jarvis dit
// sans qu'on lui parle (mail, colis, fin de tâche Claude, recherche, rappel…) passe par `notify`, puis par cette
// politique. Une annonce est reportée pendant la sourdine (visio, musique), en rafale (écart minimal) ou pendant les
// heures calmes si l'utilisateur est absent ; les rappels et programmations, demandés
// explicitement, ignorent les heures calmes. Les annonces reportées sont dites ensemble dès que c'est possible.
// Réglage config.announcements.

use std::cell::{Cell, RefCell};

use serde_json::json;

use super::{OrchRef, Up};
use crate::js;

/// Au-delà, l'utilisateur est considéré absent (les heures calmes s'appliquent).
const ABSENT_MS: i64 = 15 * 60_000;
const FLUSH_EVERY_MS: u64 = 10_000;
const MAX_SPOKEN: usize = 5;
const MAX_HELD: usize = 30;
/// Sources demandées explicitement par l'utilisateur : jamais retenues par les heures calmes.
const QUIET_EXEMPT: &[&str] = &["rappel", "programmation"];
/// Sources au contenu personnel : retenues tant que d'autres personnes sont devant l'écran (présence webcam).
const PRIVATE_SOURCES: &[&str] = &["mail", "telegram"];

/// Contexte d'une décision (tout en millisecondes ou minutes depuis minuit).
#[derive(Clone, Copy)]
pub struct Ctx {
    pub now_min: u32,
    pub quiet: Option<(u32, u32)>,
    pub min_gap_ms: i64,
    pub since_announce_ms: i64,
    pub since_user_ms: i64,
    /// Dernier visage vu par la webcam (i64::MAX : jamais, ou présence coupée).
    pub since_seen_ms: i64,
    pub muted: bool,
    pub privacy: bool,
}

/// « 22:30 » → minutes depuis minuit.
pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Plage qui peut passer minuit (22:30 → 07:30).
pub fn in_range(now: u32, (start, end): (u32, u32)) -> bool {
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

/// None : parler maintenant ; Some(motif) : reporter.
pub fn decide(source: &str, c: &Ctx) -> Option<&'static str> {
    if c.muted {
        return Some("sourdine");
    }
    if c.privacy && PRIVATE_SOURCES.contains(&source) {
        return Some("vie privée");
    }
    if !QUIET_EXEMPT.contains(&source) && c.since_user_ms.min(c.since_seen_ms) > ABSENT_MS && c.quiet.map(|q| in_range(c.now_min, q)).unwrap_or(false) {
        return Some("heures calmes");
    }
    // Rafale : une annonce suit de trop près la précédente, sans que l'utilisateur ait parlé entre-temps
    // (sinon c'est probablement la réponse à ce qu'il vient de demander).
    if c.since_announce_ms < c.min_gap_ms && c.since_user_ms > c.since_announce_ms {
        return Some("rafale");
    }
    None
}

struct Held {
    text: String,
    source: String,
}

pub struct Announcer {
    o: OrchRef,
    held: RefCell<Vec<Held>>,
    last_announce_at: Cell<i64>,
    last_user_at: Cell<i64>,
    last_seen_at: Cell<i64>,
    timer: js::Timer,
}

impl Announcer {
    pub fn new(o: OrchRef) -> Self {
        Self { o, held: RefCell::new(vec![]), last_announce_at: Cell::new(0), last_user_at: Cell::new(0), last_seen_at: Cell::new(0), timer: js::Timer::new() }
    }

    fn ctx(&self) -> Ctx {
        use chrono::Timelike;
        let o = self.o.up();
        let cfg = o.config.get("announcements");
        let now = chrono::Local::now();
        let quiet = (cfg["quietHours"] != json!(false))
            .then(|| Some((parse_hhmm(&js::gs(&cfg, "quietStart"))?, parse_hhmm(&js::gs(&cfg, "quietEnd"))?)))
            .flatten();
        let t = js::now_ms();
        let muted = o.context_mute.is_muted.get();
        Ctx {
            now_min: now.hour() * 60 + now.minute(),
            quiet,
            min_gap_ms: (js::num(&cfg["minGapSeconds"]).unwrap_or(30.0).clamp(0.0, 600.0) * 1000.0) as i64,
            since_announce_ms: t - self.last_announce_at.get(),
            since_user_ms: t - self.last_user_at.get(),
            since_seen_ms: if o.presence.enabled() && self.last_seen_at.get() > 0 { t - self.last_seen_at.get() } else { i64::MAX },
            muted,
            privacy: o.presence.in_privacy(),
        }
    }

    /// Annonce spontanée : true s'il faut la dire maintenant ; sinon elle est gardée pour plus tard.
    pub fn admit(&self, source: &str, text: &str) -> bool {
        match decide(source, &self.ctx()) {
            None => {
                self.last_announce_at.set(js::now_ms());
                true
            }
            Some(why) => {
                self.o.up().record("annonce_reportee", json!({ "source": source, "motif": why }));
                {
                    let mut held = self.held.borrow_mut();
                    held.push(Held { text: text.to_string(), source: source.to_string() });
                    if held.len() > MAX_HELD {
                        held.remove(0);
                    }
                }
                self.arm();
                false
            }
        }
    }

    /// L'utilisateur parle : ses annonces en attente lui seront dites juste après.
    pub fn user_spoke(&self) {
        self.last_user_at.set(js::now_ms());
        if !self.held.borrow().is_empty() {
            self.arm();
        }
    }

    /// Un visage est vu par la webcam : l'utilisateur est là (heures calmes levées), sans valoir une parole.
    pub fn seen(&self) {
        self.last_seen_at.set(js::now_ms());
    }

    pub fn pending(&self) -> usize {
        self.held.borrow().len()
    }

    fn arm(&self) {
        if self.timer.active() {
            return;
        }
        let w = self.o.clone();
        self.timer.every(FLUSH_EVERY_MS, move || {
            if let Some(o) = w.upgrade() {
                o.announcer.flush();
            }
        });
    }

    /// Dit ensemble les annonces qui peuvent l'être maintenant.
    pub fn flush(&self) {
        let ctx = self.ctx();
        let ready: Vec<Held> = {
            let mut held = self.held.borrow_mut();
            let all = std::mem::take(&mut *held);
            let (ready, wait): (Vec<Held>, Vec<Held>) = all.into_iter().partition(|h| decide(&h.source, &Ctx { since_announce_ms: i64::MAX, ..ctx }).is_none());
            *held = wait;
            ready
        };
        if self.held.borrow().is_empty() {
            self.timer.clear();
        }
        if ready.is_empty() || ctx.since_announce_ms < ctx.min_gap_ms {
            self.held.borrow_mut().splice(0..0, ready);
            return;
        }
        self.last_announce_at.set(js::now_ms());
        let o = self.o.up();
        o.emit(json!({ "type": "say", "text": summary(&ready), "notification": "annonces" }));
    }
}

/// Une annonce telle quelle ; plusieurs : regroupées, cinq au plus à voix haute.
fn summary(items: &[Held]) -> String {
    let texts: Vec<String> = items
        .iter()
        .map(|h| {
            let t = h.text.trim();
            if t.ends_with(['.', '!', '?', '…']) {
                t.to_string()
            } else {
                format!("{t}.")
            }
        })
        .collect();
    if texts.len() == 1 {
        return texts[0].clone();
    }
    let mut out = format!("{} annonces en attente. {}", texts.len(), texts.iter().take(MAX_SPOKEN).cloned().collect::<Vec<_>>().join(" "));
    if texts.len() > MAX_SPOKEN {
        out.push_str(&format!(" Et {} autres, dans le journal.", texts.len() - MAX_SPOKEN));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(now: &str) -> Ctx {
        Ctx {
            now_min: parse_hhmm(now).unwrap(),
            quiet: Some((parse_hhmm("22:30").unwrap(), parse_hhmm("07:30").unwrap())),
            min_gap_ms: 30_000,
            since_announce_ms: 3_600_000,
            since_user_ms: 3_600_000,
            since_seen_ms: i64::MAX,
            muted: false,
            privacy: false,
        }
    }

    #[test]
    fn les_heures_calmes_passent_minuit() {
        let q = (parse_hhmm("22:30").unwrap(), parse_hhmm("07:30").unwrap());
        assert!(in_range(parse_hhmm("23:10").unwrap(), q));
        assert!(in_range(parse_hhmm("03:00").unwrap(), q));
        assert!(!in_range(parse_hhmm("07:30").unwrap(), q));
        assert!(!in_range(parse_hhmm("12:00").unwrap(), q));
        assert_eq!(parse_hhmm("25:00"), None);
    }

    #[test]
    fn la_nuit_un_mail_attend_mais_pas_un_rappel() {
        assert_eq!(decide("mail", &ctx("02:00")), Some("heures calmes"));
        assert_eq!(decide("rappel", &ctx("02:00")), None);
        assert_eq!(decide("mail", &ctx("14:00")), None);
    }

    #[test]
    fn la_nuit_un_utilisateur_present_est_prevenu() {
        let c = Ctx { since_user_ms: 5 * 60_000, ..ctx("23:30") };
        assert_eq!(decide("mail", &c), None);
    }

    #[test]
    fn sourdine_et_rafale_reportent_mais_pas_une_reponse_immediate() {
        assert_eq!(decide("rappel", &Ctx { muted: true, ..ctx("14:00") }), Some("sourdine"));
        assert_eq!(decide("claude", &Ctx { since_user_ms: 2_000, ..ctx("23:00") }), None);
        assert_eq!(decide("mail", &Ctx { since_announce_ms: 10_000, ..ctx("14:00") }), Some("rafale"));
        assert_eq!(decide("claude", &Ctx { since_announce_ms: 10_000, since_user_ms: 4_000, ..ctx("14:00") }), None);
    }

    #[test]
    fn la_webcam_signale_la_presence_et_la_vie_privee() {
        assert_eq!(decide("mail", &Ctx { since_seen_ms: 30_000, ..ctx("23:30") }), None, "vu à l'écran : pas d'heures calmes");
        assert_eq!(decide("mail", &Ctx { privacy: true, ..ctx("14:00") }), Some("vie privée"));
        assert_eq!(decide("colis", &Ctx { privacy: true, ..ctx("14:00") }), None);
    }

    #[test]
    fn les_annonces_reportees_sont_regroupees() {
        let one = vec![Held { text: "Nouveau mail de Paul".into(), source: "mail".into() }];
        assert_eq!(summary(&one), "Nouveau mail de Paul.");
        let many: Vec<Held> = (1..=7).map(|i| Held { text: format!("Annonce {i}"), source: "mail".into() }).collect();
        let s = summary(&many);
        assert!(s.starts_with("7 annonces en attente. Annonce 1."));
        assert!(s.contains("Annonce 5.") && !s.contains("Annonce 6.") && s.ends_with("Et 2 autres, dans le journal."));
    }
}
