// Cards du HUD pour les listes à fiches. Elles passent par le widget « tool » (clé « tool:<titre> », numéro de
// card, fermeture, rafraîchissement déjà gérés) ; modules/listes/ui/listes.js les dessine d'après `collection` :
//   view = la liste (mini-cards)   item = la fiche   new = proposition de création   all = toutes les listes

use serde_json::json;

use super::super::Orchestrator;
use super::schema;
use crate::js::{self, J};

pub const OVERVIEW_TITLE: &str = "Mes listes";
pub const NEW_TITLE: &str = "Nouvelle liste";
const TTL_MS: i64 = 900_000;

pub fn list_title(list: &J) -> String {
    format!("{} {}", js::gs(list, "icone"), js::gs(list, "nom")).trim().to_string()
}

pub fn item_title(list: &J) -> String {
    format!("{} · fiche", js::gs(list, "nom"))
}

pub fn list_card(list: &J) -> J {
    let (total, owned, wish) = schema::counts(list);
    json!({
        "title": list_title(list), "tool": "liste", "collection": "view", "wide": true, "ttl": TTL_MS,
        "count": total, "possedes": owned, "souhaites": wish,
        "liste": schema::public_list(list, true)
    })
}

pub fn item_card(list: &J, element_id: &str) -> J {
    let items = js::arr(&list["elements"]);
    let idx = items.iter().position(|e| js::gs(e, "id") == element_id).unwrap_or(0);
    let item = items.get(idx).cloned().unwrap_or(J::Null);
    let neighbour = |i: Option<usize>| i.and_then(|i| items.get(i)).map(|e| js::gs(e, "id")).unwrap_or_default();
    json!({
        "title": item_title(list), "tool": "fiche", "collection": "item", "wide": true, "ttl": TTL_MS,
        "liste": schema::public_list(list, false), "element": schema::public_item(list, &item),
        "precedent": neighbour(idx.checked_sub(1)), "suivant": neighbour(Some(idx + 1)),
        "position": idx + 1, "total": items.len()
    })
}

/// Proposition de création : `keep` = clés des champs cochés.
pub fn new_card(schema_json: &J, keep: &[String]) -> J {
    let title = js::gs(schema_json, "titre");
    let fields: Vec<J> = js::arr(&schema_json["champs"])
        .into_iter()
        .map(|mut f| {
            let key = js::gs(&f, "cle");
            f["retenu"] = json!(keep.contains(&key) || key == title);
            f["fixe"] = json!(key == title);
            f
        })
        .collect();
    json!({
        "title": NEW_TITLE, "tool": "liste", "collection": "new", "wide": true, "ttl": TTL_MS,
        "proposition": { "nom": schema_json["nom"], "icone": schema_json["icone"], "modele": schema_json["modele"], "noms": schema_json["noms"], "champs": fields }
    })
}

/// Toutes les listes : à fiches (collections) et à cocher (listes intelligentes).
pub fn overview_card(o: &Orchestrator) -> J {
    let mut fiches: Vec<J> = o.collections.all().iter().map(|l| schema::public_list(l, false)).collect();
    fiches.sort_by_key(|l| std::cmp::Reverse(js::gs(l, "majLe")));
    let mut cocher: Vec<J> = o
        .smart_lists
        .lists
        .borrow()
        .iter()
        .map(|l| {
            let items = js::arr(&l["items"]);
            let done = items.iter().filter(|i| js::gb(i, "checked")).count();
            json!({ "id": l["id"], "nom": l["title"], "total": items.len(), "coches": done, "majLe": l["updatedAt"] })
        })
        .collect();
    cocher.sort_by_key(|l| std::cmp::Reverse(js::gs(l, "majLe")));
    let models: Vec<J> = schema::presets().iter().filter(|p| js::gs(p, "id") != "libre").map(|p| json!({ "id": p["id"], "nom": p["nom"], "icone": p["icone"] })).collect();
    json!({
        "title": OVERVIEW_TITLE, "tool": "listes", "collection": "all", "wide": true, "ttl": TTL_MS,
        "count": fiches.len() + cocher.len(), "fiches": fiches, "cocher": cocher, "modeles": models
    })
}
