// Qualité de la reconnaissance vocale : combien de phrases ont été comprises, rejetées ou acceptées de justesse,
// d'après le journal scellé (aucune mesure supplémentaire). Alimente la page « Qualité vocale » de l'administration
// et le voyant « Oreilles » du checkup.

use serde_json::json;

use super::Orchestrator;
use crate::js::{self, J};

const DAY_MS: i64 = 24 * 3600 * 1000;
/// Types du journal qui comptent : (type, clé du compteur).
const KINDS: &[(&str, &str)] = &[
    ("demande", "comprises"),
    ("voix_proche_acceptee", "prochesAcceptees"),
    ("voix_non_reconnue", "voixRejetees"),
    ("demande_ignoree", "ignorees"),
    ("parole_parasite_ignoree", "parasites"),
];
/// En dessous de ce nombre de phrases sur 24 h, le taux de rejet n'est pas significatif.
const MIN_SAMPLE: i64 = 10;
const WARN_RATE: f64 = 0.4;

impl Orchestrator {
    /// Compteurs sur 24 h et 7 jours, taux de rejet et dernières phrases écartées.
    pub fn voice_quality(&self) -> J {
        let now = js::now_ms();
        let mut d1 = serde_json::Map::new();
        let mut d7 = serde_json::Map::new();
        for (_, key) in KINDS {
            d1.insert((*key).into(), json!(0));
            d7.insert((*key).into(), json!(0));
        }
        let mut recent: Vec<J> = vec![];
        for r in self.history.read(false).iter().rev() {
            let Some(at) = js::parse_date_ms(&js::gs(r, "at")) else { continue };
            let age = now - at;
            if age > 7 * DAY_MS {
                break;
            }
            let typ = js::gs(r, "type");
            let Some((_, key)) = KINDS.iter().find(|(t, _)| *t == typ) else { continue };
            for (map, limit) in [(&mut d1, DAY_MS), (&mut d7, 7 * DAY_MS)] {
                if age <= limit {
                    let n = map[*key].as_i64().unwrap_or(0) + 1;
                    map.insert((*key).into(), json!(n));
                }
            }
            if typ != "demande" && recent.len() < 25 {
                let data = &r["data"];
                recent.push(json!({
                    "at": r["at"], "type": typ, "texte": js::slice_chars(&js::gs(data, "texte"), 160),
                    "similarite": data["similarite"], "seuil": data["seuil"], "verdict": data["verdict"],
                    "proche": data["proche"], "motif": data["motif"]
                }));
            }
        }
        let (rate, sample) = Self::reject_rate(&d1);
        let speaker = self.get_speaker_status();
        json!({
            "ok": true,
            "h24": d1, "j7": d7,
            "tauxRejet24h": rate, "echantillon24h": sample, "significatif": sample >= MIN_SAMPLE,
            "reglages": {
                "seuil": speaker["threshold"], "tolerance": speaker["nearMiss"], "margeDoute": speaker["doubtMargin"],
                "mode": speaker["mode"], "voixConnues": js::arr(&speaker["speakers"]).len()
            },
            "recentes": recent
        })
    }

    /// Part des phrases écartées (voix rejetée, ignorée ou parasite) parmi les phrases entendues, et leur nombre.
    fn reject_rate(counts: &serde_json::Map<String, J>) -> (f64, i64) {
        let n = |k: &str| counts.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
        let ok = n("comprises") + n("prochesAcceptees");
        let bad = n("voixRejetees") + n("ignorees") + n("parasites");
        let total = ok + bad;
        (if total == 0 { 0.0 } else { (bad as f64 / total as f64 * 1000.0).round() / 1000.0 }, total)
    }

    /// Résumé pour le checkup : (état, phrase). Vide tant que l'échantillon est trop petit.
    pub fn voice_quality_hint(&self) -> Option<(&'static str, String)> {
        let q = self.voice_quality();
        let sample = q["echantillon24h"].as_i64().unwrap_or(0);
        if sample < MIN_SAMPLE {
            return None;
        }
        let rate = q["tauxRejet24h"].as_f64().unwrap_or(0.0);
        let text = format!("{} % des phrases écartées en 24 h ({sample} entendues).", (rate * 100.0).round() as i64);
        Some((if rate >= WARN_RATE { "warn" } else { "ok" }, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taux_de_rejet() {
        let mut m = serde_json::Map::new();
        for (k, v) in [("comprises", 6), ("prochesAcceptees", 2), ("voixRejetees", 1), ("ignorees", 1), ("parasites", 0)] {
            m.insert(k.into(), json!(v));
        }
        assert_eq!(Orchestrator::reject_rate(&m), (0.2, 10));
        assert_eq!(Orchestrator::reject_rate(&serde_json::Map::new()), (0.0, 0));
    }
}
