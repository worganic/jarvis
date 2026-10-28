// Commandes vocales des dossiers « Savoir » : créer (avec un temps de recherche), approfondir, ouvrir, statistiques,
// supprimer (en deux temps). Un dossier se désigne par des mots de son sujet ; une phrase qui ne désigne aucun
// dossier connu (« ouvre le dossier Documents ») est laissée aux autres commandes.

use serde_json::json;

use super::{Savoir, DEFAULT_MINUTES};
use crate::js;
use crate::orch::intent::Intent;
use crate::orch::Up;

const DOSSIER: &str = r"(?:le |mon |ce )?dossier (?:data )?(?:sur |de |du |des |d |a propos de |concernant )?";

fn minutes_of(word: &str) -> Option<u32> {
    if let Ok(n) = word.parse() {
        return Some(n);
    }
    Some(match word {
        "une" | "un" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "quinze" => 15,
        "vingt" => 20,
        "trente" => 30,
        _ => return None,
    })
}

/// « … en 10 minutes » en fin de phrase : (phrase sans la durée, minutes).
pub fn split_duration(text: &str) -> (String, Option<u32>) {
    let re = crate::re!(r"(?i)[\s,]+(?:en|pendant|sur|durant)\s+(\d{1,2}|une?|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|quinze|vingt|trente)\s+(?:minutes?|min|mn)[\s.!?]*$");
    match re.captures(text) {
        Some(m) => (text[..m.get(0).unwrap().start()].trim().to_string(), minutes_of(&m[1].to_lowercase())),
        None => (text.trim().trim_end_matches(['.', '!', '?']).trim().to_string(), None),
    }
}

/// Sujet tel qu'il a été dit, après « sur / à propos de… ».
pub fn topic_from(raw: &str) -> Option<String> {
    let m = crate::re!(r"(?i)\b(?:dossier|fiche|base de connaissances?)\b[^\n]*?\b(?:sur|à propos de|à propos des|a propos de|concernant|au sujet de|au sujet des)\s+(.+)$").captures(raw)?;
    let (t, _) = split_duration(&m[1]);
    let t = crate::re!(r"(?i)^(?:l['’]\s*|(?:le|la|les|un|une|des|du)\s+)").replace(&t, "").to_string();
    Some(t).filter(|t| t.chars().count() >= 2)
}

impl Savoir {
    pub fn match_(&self, t: &str, raw: &str) -> Option<Intent> {
        let o = self.o.clone();

        // Créer : « fais un dossier sur l'hélicoptère H160 en 10 minutes », « ajoute le H160 à la data ».
        let create = crate::re!(r"^(?:fais|fait|prepare|constitue|monte|lance|compile|realise)(?: moi)? (?:un |une )?(?:dossier|fiche|base de connaissances?)(?: complet| complete| documentaire| data| de recherche)? (?:sur|a propos de|a propos des|concernant|au sujet de|au sujet des) (.+)$");
        let add = crate::re!(r"^(?:ajoute|mets|met)(?: moi)? (.+?) (?:a|dans) (?:la |ma )?(?:liste |card )?data$");
        let topic = if create.is_match(t) {
            topic_from(raw)
        } else if let Some(m) = add.captures(t) {
            let raw_m = crate::re!(r"(?i)^\s*(?:ajoute|mets|met)(?:-moi| moi)?\s+(.+?)\s+(?:à|a|dans)\s+(?:la |ma )?(?:liste |card )?data[\s.!?]*$").captures(raw);
            Some(raw_m.map(|r| r[1].to_string()).unwrap_or_else(|| m[1].to_string()))
        } else {
            None
        };
        if let Some(topic) = topic {
            let (_, minutes) = split_duration(raw);
            let folded = js::fold_lower(&topic);
            if crate::re!(r"^(?:le |mon |sur le )?(?:bureau|disque|cle usb|desktop|telechargements)$").is_match(&folded) {
                return None; // un dossier Windows, pas un sujet de recherche
            }
            let minutes = minutes.unwrap_or(DEFAULT_MINUTES);
            return Some(Intent::new("savoir", "Dossier Data", &topic.clone(), move || {
                let (o, topic) = (o.clone(), topic.clone());
                async move {
                    let orch = o.up();
                    let slug = orch.savoir.find(&topic).filter(|s| same_subject(&orch.savoir, s, &topic));
                    let r = orch.savoir.start(&topic, slug.as_deref(), "", minutes);
                    json!({ "say": r["say"], "display": r["display"], "quiet": true })
                }
            }));
        }

        // Liste : « ouvre la data », « ouvre le module savoir », « quels sont mes dossiers ».
        if crate::re!(r"^(?:(?:ouvre|ouvrez|ouvrir|affiche|affichez|afficher|montre|montrez|montrer|lance|lancez|lancer|demarre|active|liste|donne)(?: moi)? (?:la |le |ma |mes |les )?(?:(?:liste|card|cards|carte|cartes|carde|cardes|module|modules|widget|fiche) (?:de |des |d )?)?(?:data|savoir|dossiers? data|dossiers? de recherche)(?: \d{1,3})?|quels sont mes dossiers(?: data| de recherche)?)$").is_match(t) {
            return Some(Intent::new("savoir", "Dossiers Data", "liste", move || {
                let o = o.clone();
                async move {
                    let orch = o.up();
                    orch.savoir.show_list();
                    let n = orch.savoir.list().len();
                    json!({ "say": if n == 0 { "Tu n'as encore aucun dossier. Dis par exemple : fais un dossier sur l'hélicoptère H160.".to_string() } else { format!("Tu as {n} dossier{}.", if n > 1 { "s" } else { "" }) }, "display": "Dossiers Data" })
                }
            }));
        }

        // Approfondir : « complète le dossier H160 avec les versions militaires (en 5 minutes) ».
        if let Some(m) = crate::re!(&format!(r"^(?:complete|approfondis|enrichis|ameliore|continue|relance)(?: moi)? {DOSSIER}(.+?)(?: (?:avec|en cherchant|concernant|a propos de|au sujet de|sur) (.+))?$")).captures(t) {
            let (name, _) = split_duration(&m[1]);
            let slug = self.find(&name)?;
            let axe = m.get(2).map(|_| {
                let raw_axe = crate::re!(r"(?i)\s(?:avec|en cherchant|concernant|à propos de|a propos de|au sujet de|sur)\s+(.+)$").captures(raw).map(|c| c[1].to_string()).unwrap_or_default();
                split_duration(&raw_axe).0
            });
            let (_, minutes) = split_duration(raw);
            let minutes = minutes.unwrap_or(DEFAULT_MINUTES);
            return Some(Intent::new("savoir", "Approfondir un dossier", &slug.clone(), move || {
                let (o, slug, axe) = (o.clone(), slug.clone(), axe.clone().unwrap_or_default());
                async move {
                    let orch = o.up();
                    let sujet = orch.savoir.get(&slug).map(|d| js::gs(&d, "sujet")).unwrap_or_default();
                    let r = orch.savoir.start(&sujet, Some(&slug), &axe, minutes);
                    json!({ "say": r["say"], "display": r["display"], "quiet": true })
                }
            }));
        }

        // Supprimer, en deux temps : « supprime le dossier H160 » puis « supprime définitivement le dossier H160 ».
        if let Some(m) = crate::re!(&format!(r"^(?:supprime|efface|jette|detruis)( definitivement)? {DOSSIER}(.+)$")).captures(t) {
            let slug = self.find(&m[2])?;
            let sure = m.get(1).is_some();
            return Some(Intent::new("savoir", "Supprimer un dossier", &slug.clone(), move || {
                let (o, slug) = (o.clone(), slug.clone());
                async move {
                    let orch = o.up();
                    let sujet = orch.savoir.get(&slug).map(|d| js::gs(&d, "sujet")).unwrap_or_default();
                    if !sure {
                        return json!({ "say": format!("Pour confirmer, dis : supprime définitivement le dossier {sujet}."), "display": format!("Supprimer {sujet} ?") });
                    }
                    let ok = orch.savoir.delete(&slug);
                    json!({ "say": if ok { format!("Dossier {sujet} supprimé.") } else { "Je ne peux pas le supprimer pendant qu'une recherche est en cours.".to_string() }, "display": format!("Dossier {sujet}") })
                }
            }));
        }

        // Statistiques et ouverture d'un dossier.
        let stats = crate::re!(&format!(r"^(?:(?:donne moi |dis moi )?(?:les )?(?:statistiques?|stats|chiffres|etat|infos?)(?: du| de mon| sur le| sur mon)? {DOSSIER}(.+)|(?:ouvre|affiche|montre)(?: moi)? {DOSSIER}(.+))$")).captures(t);
        if let Some(m) = stats {
            let is_stats = m.get(1).is_some();
            let slug = self.find(m.get(1).or(m.get(2)).map(|x| x.as_str()).unwrap_or(""))?;
            return Some(Intent::new("savoir", "Dossier Data", &slug.clone(), move || {
                let (o, slug) = (o.clone(), slug.clone());
                async move {
                    let orch = o.up();
                    let Some(d) = orch.savoir.detail(&slug) else { return json!({ "say": "Dossier introuvable." }) };
                    orch.savoir.show(&slug);
                    let s = &d["stats"];
                    let say = if is_stats {
                        format!(
                            "Le dossier {} compte {} sources, {} sections, {} images et {} passages indexés, après {} recherche{}. Tu l'as interrogé {} fois.",
                            js::gs(&d, "sujet"), s["sources"], s["sections"], s["images"], d["passages"], s["recherches"], if s["recherches"].as_i64().unwrap_or(0) > 1 { "s" } else { "" }, s["questions"]
                        )
                    } else {
                        format!("Voici le dossier {}.", js::gs(&d, "sujet"))
                    };
                    json!({ "say": say, "display": format!("Dossier {}", js::gs(&d, "sujet")) })
                }
            }));
        }

        // Explorateur 3D : « montre-moi le H160 en 3D », « ouvre la maquette du H160 », « explore le H160 ».
        let in_3d = crate::re!(r"^(?:montre|affiche|ouvre|fais(?: moi)? voir)(?: moi| nous)? (?:le |la |les |l )?(.+?) en 3 ?d$").captures(t);
        let model_of = crate::re!(r"^(?:montre|affiche|ouvre)(?: moi| nous)? (?:la )?(?:maquette|vue 3 ?d|modele 3 ?d) (?:du |de la |des |de l |de |d )(.+)$").captures(t);
        let explore_it = crate::re!(r"^explore(?: le| la| les| l)? (.+?)(?: en 3 ?d)?$").captures(t);
        if let Some(m) = in_3d.or(model_of).or(explore_it) {
            let slug = self.find(&m[1])?;
            return Some(Intent::new("savoir", "Explorateur 3D", &slug.clone(), move || {
                let (o, slug) = (o.clone(), slug.clone());
                async move {
                    let r = o.up().savoir.explore(&slug, None).await;
                    json!({ "say": r["say"], "display": r["display"] })
                }
            }));
        }

        // Présentation animée : « présente-moi le H160 », « fais-moi une présentation de l'hélicoptère H160 ».
        if let Some(m) = crate::re!(r"^(?:presente|presentes|fais(?: moi| nous)? une presentation|lance(?: moi)? une presentation|montre)(?: moi| nous)? (?:(?:du |de la |des |de l |de |d |sur le |sur la |sur les |sur l |sur )?)(.+)$").captures(t) {
            let name = m[1].trim_end_matches(" en grand").to_string();
            if name.starts_with("dossier") || name.starts_with("le dossier") {
                return None;
            }
            let slug = self.find(&name)?;
            return Some(Intent::new("savoir", "Présentation", &slug.clone(), move || {
                let (o, slug) = (o.clone(), slug.clone());
                async move {
                    let orch = o.up();
                    let r = orch.savoir.present(&slug).await;
                    if js::truthy(&r["say"]) {
                        json!({ "say": r["say"], "display": "Présentation" })
                    } else {
                        r
                    }
                }
            }));
        }
        None
    }
}

/// Un dossier existant ne reprend une demande de création que s'il porte vraiment sur le même sujet.
pub(super) fn same_subject(s: &Savoir, slug: &str, topic: &str) -> bool {
    let want = super::words(topic);
    let have = super::words(&format!("{slug} {}", s.get(slug).map(|d| js::gs(&d, "sujet")).unwrap_or_default()));
    !want.is_empty() && want.iter().all(|w| have.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duree_et_sujet() {
        assert_eq!(split_duration("Fais un dossier sur le H160 en 10 minutes."), ("Fais un dossier sur le H160".into(), Some(10)));
        assert_eq!(split_duration("dossier sur le H160 pendant cinq min"), ("dossier sur le H160".into(), Some(5)));
        assert_eq!(split_duration("dossier sur le H160"), ("dossier sur le H160".into(), None));
        assert_eq!(topic_from("Fais-moi un dossier complet sur l'hélicoptère Airbus H160 en 3 minutes").as_deref(), Some("hélicoptère Airbus H160"));
        assert_eq!(topic_from("prépare une fiche à propos des batteries au sodium").as_deref(), Some("batteries au sodium"));
    }

    #[test]
    fn un_dossier_existant_est_designe_par_ses_mots() {
        let dir = std::env::temp_dir().join(format!("jarvis-test-savoir-{}", js::now_ms()));
        std::fs::create_dir_all(dir.join("helicoptere-airbus-h160")).unwrap();
        std::fs::write(dir.join("helicoptere-airbus-h160").join("dossier.json"), r#"{ "sujet": "Hélicoptère Airbus H160", "meta": {} }"#).unwrap();
        let s = Savoir::new(std::rc::Weak::new(), dir.clone(), std::rc::Rc::new(|_| {}));
        let kind = |raw: &str| s.match_(&crate::orch::router::fold(raw), raw).map(|i| i.label);
        assert_eq!(s.find("le H160").as_deref(), Some("helicoptere-airbus-h160"));
        assert_eq!(kind("Ouvre le dossier H160").as_deref(), Some("Dossier Data"));
        assert_eq!(kind("Complète le dossier H160 avec les versions militaires").as_deref(), Some("Approfondir un dossier"));
        assert_eq!(kind("Supprime le dossier H160").as_deref(), Some("Supprimer un dossier"));
        assert_eq!(kind("Statistiques du dossier H160").as_deref(), Some("Dossier Data"));
        assert!(kind("Ouvre le dossier documents").is_none(), "un dossier Windows reste aux autres commandes");
        assert!(kind("Supprime le dossier temp").is_none());
        assert_eq!(kind("Présente-moi le H160").as_deref(), Some("Présentation"));
        assert_eq!(kind("Fais-moi une présentation de l'hélicoptère H160").as_deref(), Some("Présentation"));
        assert!(kind("Présente-toi").is_none());
        assert_eq!(kind("Montre-moi le H160 en 3D").as_deref(), Some("Explorateur 3D"));
        assert_eq!(kind("Ouvre la maquette du H160").as_deref(), Some("Explorateur 3D"));
        assert_eq!(kind("Explore le H160").as_deref(), Some("Explorateur 3D"));
        assert!(same_subject(&s, "helicoptere-airbus-h160", "H160 Airbus"));
        assert!(!same_subject(&s, "helicoptere-airbus-h160", "H160 militaire"), "un autre sujet donne un autre dossier");
        let _ = std::fs::remove_dir_all(dir);
    }
}
