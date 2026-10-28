// Cache des réponses factuelles (orchestrator/answer-cache.js) : la même question générale, reposée
// dans les 30 minutes, reçoit la même réponse sans nouvel appel à l'IA. Jamais mis en cache : ce qui
// dépend du moment, de l'utilisateur, ou une tâche qui a utilisé des outils.

use std::cell::RefCell;

use crate::js;

pub const TTL_MS: i64 = 30 * 60 * 1000;
pub const MAX: usize = 100;

pub fn fold(text: &str) -> String {
    let t = js::fold_lower(text);
    let t = crate::re!(r#"['’\-?!,;:.…"«»]"#).replace_all(&t, " ");
    let t = crate::re!(r"\s+").replace_all(&t, " ").trim().to_string();
    let t = crate::re!(r"^(?:jarvis |dis moi |dis |est ce que tu sais |tu sais |sais tu )").replace(&t, "").into_owned();
    crate::re!(r"\s+(?:s il te plait|stp|jarvis)$").replace(&t, "").into_owned()
}

#[derive(Clone)]
pub struct Cached {
    pub answer: String,
    pub at: i64,
}

pub struct AnswerCache {
    ttl_ms: i64,
    map: RefCell<Vec<(String, Cached)>>,
}

impl Default for AnswerCache {
    fn default() -> Self {
        Self { ttl_ms: TTL_MS, map: RefCell::new(vec![]) }
    }
}

impl AnswerCache {
    pub fn is_cacheable(&self, question: &str) -> bool {
        let q = fold(question);
        let question_re = crate::re!(r"^(?:c est quoi|qu est ce que|qu est ce qu|qui (?:est|etait|a|sont|etaient)|quel(?:le)?s? (?:est|sont|etait)|combien (?:de|d|y a t il|mesure|pese)|comment (?:dit on|s appelle|fonctionne|marche)|pourquoi|que (?:veut dire|signifie)|definis?|definition|ou (?:se trouve|est situe|sont situes)|en quelle annee|quand (?:a ete|est ne|est mort|a eu lieu))\b");
        let volatile = crate::re!(r"\b(?:aujourd hui|demain|hier|maintenant|en ce moment|actuel(?:le)?s?|dernier|derniere|derniers|recent|recemment|cette (?:semaine|annee)|ce (?:soir|matin|mois)|heure|date|meteo|temps qu il fait|prix|cours|score|match|resultat|mon|ma|mes|moi|je|j|tu|ton|ta|tes|toi|nous|notre|nos|ici|son|sa|ses|leur|leurs|lui|ce dernier|cette personne|celui ci|celle ci)\b");
        question_re.is_match(&q)
            && !volatile.is_match(&q)
            && q.chars().count() <= 200
            && q.split(' ').count() >= 4
            && !crate::re!(r"^(?:et|mais|alors|donc) ").is_match(&q)
            && !crate::re!(r"\b(?:ca|cela|ce|celui|celle|il|elle|ils|elles|le|la|les)$").is_match(&q)
    }

    pub fn len(&self) -> usize {
        self.map.borrow().len()
    }

    pub fn get(&self, question: &str) -> Option<Cached> {
        let key = fold(question);
        let mut map = self.map.borrow_mut();
        let idx = map.iter().position(|(k, _)| *k == key)?;
        if js::now_ms() - map[idx].1.at > self.ttl_ms {
            map.remove(idx);
            return None;
        }
        Some(map[idx].1.clone())
    }

    /// used_tools : la tâche a appelé des outils (fichiers, web, mails…) → pas de cache.
    pub fn put(&self, question: &str, answer: &str, used_tools: bool) -> bool {
        if used_tools || answer.is_empty() || answer.chars().count() > 1500 || !self.is_cacheable(question) {
            return false;
        }
        let key = fold(question);
        let mut map = self.map.borrow_mut();
        map.retain(|(k, _)| *k != key);
        map.push((key, Cached { answer: answer.to_string(), at: js::now_ms() }));
        while map.len() > MAX {
            map.remove(0);
        }
        true
    }
}
