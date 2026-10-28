// Séries : TVmaze (gratuit, sans clé). Titre exact, année de première diffusion, genre, chaîne ou plateforme,
// saisons, épisodes, état de la diffusion, synopsis et affiche. Les films n'ont pas de catalogue gratuit sans clé
// qui fonctionne : ils passent par la recherche web de l'IA (enrich.rs, fournisseur « web »).

use std::time::Duration;

use serde_json::{json, Map, Value};

use super::enrich::{download_image, title_close, PERSONAL};
use super::schema;
use super::service::Collections;
use crate::js::{self, J};

const USER_AGENT: &str = "Jarvis/4.19 (assistant personnel local)";

async fn get_json(url: &str) -> Result<J, String> {
    let resp = crate::net::http().get(url).header("User-Agent", USER_AGENT).timeout(Duration::from_secs(20)).send().await.map_err(|e| format!("réseau : {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("TVmaze a répondu {}", resp.status().as_u16()));
    }
    resp.json().await.map_err(|e| format!("réponse illisible : {e}"))
}

/// Texte d'un résumé HTML : sans balises, entités courantes décodées.
pub fn strip_html(s: &str) -> String {
    let t = crate::re!(r"<[^>]*>").replace_all(s, " ");
    let t = t.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&rsquo;", "’").replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">");
    schema::clean_text(&t, 800)
}

fn https(url: &str) -> String {
    url.replacen("http://", "https://", 1)
}

/// Les séries d'une réponse TVmaze : { id, titre, score, valeurs{…}, image }.
pub fn parse_tvmaze(v: &J) -> Vec<J> {
    let mut out: Vec<J> = vec![];
    for r in js::arr(v) {
        let s = &r["show"];
        let Some(id) = s["id"].as_i64() else { continue };
        let genres: Vec<String> = js::arr(&s["genres"]).iter().take(3).map(js::s).collect();
        let channel = if js::truthy(&s["network"]["name"]) { js::gs(&s["network"], "name") } else { js::gs(&s["webChannel"], "name") };
        let diffusion = match js::gs(s, "status").as_str() {
            "Running" => "En cours",
            "Ended" => "Terminée",
            "To Be Determined" | "In Development" => "À venir",
            _ => "",
        };
        let mut vals = Map::new();
        if let Some(y) = js::gs(s, "premiered").get(..4).filter(|y| y.parse::<i64>().is_ok()) {
            vals.insert("annee".into(), json!(y));
        }
        if !genres.is_empty() {
            vals.insert("genre".into(), json!(genres.join(", ")));
        }
        if !channel.is_empty() {
            vals.insert("plateforme".into(), json!(channel));
        }
        if !diffusion.is_empty() {
            vals.insert("diffusion".into(), json!(diffusion));
        }
        let summary = strip_html(&js::gs(s, "summary"));
        if !summary.is_empty() {
            vals.insert("synopsis".into(), json!(summary));
        }
        let image = js::gs(&s["image"], "original");
        let image = if image.is_empty() { js::gs(&s["image"], "medium") } else { image };
        out.push(json!({
            "id": id, "titre": schema::clean_text(&js::gs(s, "name"), 120), "score": (r["score"].as_f64().unwrap_or(0.0) * 100.0).round().min(100.0) as i64,
            "valeurs": Value::Object(vals), "image": https(&image)
        }));
        if out.len() >= 5 {
            break;
        }
    }
    out
}

/// (saisons, épisodes) d'une réponse « shows/<id>?embed=seasons ».
pub fn count_seasons(v: &J) -> (usize, i64) {
    let seasons = js::arr(&v["_embedded"]["seasons"]);
    (seasons.len(), seasons.iter().map(|s| s["episodeOrder"].as_i64().unwrap_or(0)).sum())
}

impl Collections {
    /// TVmaze. Some(résumé) si la fiche a été complétée.
    pub(super) async fn enrich_series(&self, list: &J, item: &J, opts: super::enrich::EnrichOpts) -> Result<Option<J>, String> {
        let title = schema::display(list, item, &js::gs(list, "titre"));
        let mut cands = js::arr(&item["candidats"]);
        let mut idx = item["candidatIndex"].as_u64().unwrap_or(0) as usize;
        if opts.suivant && !cands.is_empty() {
            idx = (idx + 1) % cands.len();
        } else if cands.is_empty() || opts.ecraser {
            cands = parse_tvmaze(&get_json(&format!("https://api.tvmaze.com/search/shows?q={}", urlencoding::encode(&title))).await?);
            idx = 0;
        }
        let Some(c) = cands.get(idx).cloned() else { return Ok(None) };
        let mut values = c["valeurs"].as_object().cloned().unwrap_or_default();
        // Saisons et épisodes : un second appel pour la série choisie (échec sans gravité).
        if let Ok(d) = get_json(&format!("https://api.tvmaze.com/shows/{}?embed=seasons", c["id"].as_i64().unwrap_or(0))).await {
            let (seasons, episodes) = count_seasons(&d);
            if seasons > 0 {
                values.insert("saisons".into(), json!(seasons));
            }
            if episodes > 0 {
                values.insert("episodes".into(), json!(episodes));
            }
        }
        let mut patch = Map::new();
        for (k, v) in &values {
            if PERSONAL.contains(&k.as_str()) {
                continue;
            }
            if let Some(f) = schema::field(list, k) {
                if js::gs(f, "type") != "image" {
                    if let Ok(nv) = schema::normalize_value(f, v) {
                        if !nv.is_null() {
                            patch.insert(k.clone(), nv);
                        }
                    }
                }
            }
        }
        let image = match c["image"].as_str().filter(|u| !u.is_empty()) {
            Some(url) => match download_image(url, false).await {
                Ok((bytes, ext)) => self.store_image(&js::gs(list, "id"), &js::gs(item, "id"), &bytes, ext).ok(),
                Err(_) => None,
            },
            None => None,
        };
        let close = title_close(&title, &js::gs(&c, "titre"));
        let conf = if close { 0.9 - 0.1 * idx as f64 } else { 0.4 };
        let meta = json!({ "origine": "TVmaze", "confiance": (conf.max(0.1) * 100.0).round() / 100.0, "candidats": cands, "candidatIndex": idx });
        self.apply_enrichment(&js::gs(list, "id"), &js::gs(item, "id"), patch, image.clone(), meta, opts.ecraser)?;
        Ok(Some(json!({ "artiste": values.get("plateforme"), "annee": values.get("annee"), "image": image.is_some() })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reponse_tvmaze() {
        let data = json!([{ "score": 1.18, "show": {
            "id": 169, "name": "Breaking Bad", "genres": ["Drama", "Crime", "Thriller", "Autre"], "status": "Ended", "premiered": "2008-01-20",
            "network": { "name": "AMC" }, "webChannel": null, "summary": "<p><b>Breaking Bad</b> suit un prof &amp; son ancien élève.</p>",
            "image": { "original": "http://static.tvmaze.com/uploads/o.jpg", "medium": "https://static.tvmaze.com/uploads/m.jpg" } } },
            { "show": { "name": "Sans identifiant" } }]);
        let c = parse_tvmaze(&data);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0]["titre"], json!("Breaking Bad"));
        assert_eq!(c[0]["valeurs"]["annee"], json!("2008"));
        assert_eq!(c[0]["valeurs"]["genre"], json!("Drama, Crime, Thriller"));
        assert_eq!(c[0]["valeurs"]["plateforme"], json!("AMC"));
        assert_eq!(c[0]["valeurs"]["diffusion"], json!("Terminée"));
        assert_eq!(c[0]["valeurs"]["synopsis"], json!("Breaking Bad suit un prof & son ancien élève."));
        assert_eq!(c[0]["image"], json!("https://static.tvmaze.com/uploads/o.jpg"), "l'image passe en https");
    }

    #[test]
    fn saisons_et_episodes() {
        let d = json!({ "_embedded": { "seasons": [{ "episodeOrder": 7 }, { "episodeOrder": 13 }, { "episodeOrder": null }] } });
        assert_eq!(count_seasons(&d), (3, 20));
    }

    #[test]
    fn html_neutralise() {
        assert_eq!(strip_html("<script>alert(1)</script><p>Texte&nbsp;&lt;ok&gt;</p>"), "alert(1) Texte <ok>");
    }

    /// Essai réel (réseau) : `cargo test -p jarvis-core serie_reelle -- --ignored`.
    #[test]
    #[ignore]
    fn serie_reelle() {
        let dir = std::env::temp_dir().join(format!("jarvis-test-serie-{}", std::process::id()));
        let svc = Collections::new(std::rc::Weak::<crate::orch::Orchestrator>::new(), dir.clone(), std::rc::Rc::new(|_| {}));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            let r = svc.dispatch("creer_liste", &schema::preset("serie").unwrap(), "test").await;
            assert!(js::gb(&r, "ok"), "{r}");
            let lid = js::gs(&r["liste"], "id");
            let a = svc.dispatch("ajouter_element", &json!({ "liste": lid, "valeurs": { "titre": "Breaking Bad" } }), "test").await;
            let eid = js::gs(&a["element"], "id");
            let e = svc.dispatch("enrichir", &json!({ "liste": lid, "element": eid }), "test").await;
            assert!(js::gb(&e, "ok"), "{e}");
            let list = svc.get(&lid).unwrap();
            let item = js::arr(&list["elements"])[0].clone();
            println!("{}", js::pretty(&item["valeurs"]));
            assert_eq!(item["valeurs"]["annee"], json!(2008));
            assert_eq!(item["valeurs"]["saisons"], json!(5));
            assert!(js::gs(&item, "image").starts_with("local:"));
        });
        let _ = std::fs::remove_dir_all(dir);
    }
}
