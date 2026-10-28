// Voir et piloter le bureau à la voix (orchestrator/desktop-intents.js) : cards « Bureau » (fenêtres par
// écran, applications en arrière-plan) et navigateurs (instances numérotées, pages « 2.1 »), commandes
// afficher, réduire, agrandir, déplacer d'écran, centrer, gauche / droite, fermer ; onglets de Chrome
// (extension) ; clics et saisies dans les applications (UI Automation, OCR en secours).
// Les actions passent par local_tool : mcp__system__fenetre (libre), mcp__system__fenetre_fermer (accord oral).

use std::rc::Rc;

use serde_json::{json, Value};

use super::desktop_state::{browser_of, fold, DesktopState};
use super::intent::Intent;
use super::ui_control::forbidden_name;
use super::window_slots::SLOTS;
use super::{Fut, OrchRef, Orchestrator, Up};
use crate::js::{self, J};

const BROWSER_KEYS: [&str; 5] = ["chrome", "msedge", "brave", "firefox", "opera"];
const WINDOWS: &str = "(?:applis?|applications?|fenetres?|logiciels?|programmes?)";
const OPEN_ADJ: &str = "(?:ouvertes?|ouverts?|lancees?|lances?|en cours|actives?|affichees?|qui tournent)";
const PC: &str = "(?: sur (?:mon |le |l )?(?:pc|ordi|ordinateur|bureau))?";
const SCREEN: &str = r"((?:l |le |mon )?(?:autre ecran|ecran (?:\d|un|deux|trois|principal|secondaire|de gauche|de droite)|moniteur (?:\d|un|deux|principal|secondaire)))";
const VERB_SHOW: &str = "(?:affiche|montre|active|passe sur|bascule sur|va sur|reviens sur|mets au premier plan|ramene)";
const BROWSER: &str = "(chrome|edge|firefox|brave|opera|(?:le |mon )?navigateur)";
const N: &str = r"(\d{1,2}|un|une|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|onze|douze|treize|quatorze|quinze|seize|vingt)";

macro_rules! lazy_re {
    ($name:ident, $build:expr) => {
        fn $name() -> &'static regex::Regex {
            static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            R.get_or_init(|| regex::Regex::new(&$build).unwrap())
        }
    };
}

lazy_re!(list_apps_re, format!("^(?:quelles? (?:sont )?(?:les |mes )?{WINDOWS}(?: qui)?(?: sont)?(?: {OPEN_ADJ})?{PC}|(?:liste|montre|affiche|donne|dis)(?: moi)? (?:les |mes |toutes les )?{WINDOWS}(?: {OPEN_ADJ})?{PC}|(?:(?:fais|donne|affiche|montre|dis)(?: moi)? )?(?:la |une )?liste (?:des|de mes|de toutes les) {WINDOWS}(?: {OPEN_ADJ})?{PC}|quelles? {WINDOWS} (?:tournent|sont en cours){PC}|qu est ce qui (?:est ouvert|tourne){PC}|(?:l )?etat (?:du|de mon) (?:bureau|pc|ordi)|mon bureau)$"));
lazy_re!(on_screen_re, format!("^(?:qu est ce qu il y a|qu y a t il|qu est ce qui est (?:affiche|ouvert)|quelles? {WINDOWS}(?: sont| il y a)?(?: ouvertes?| affichees?)?|(?:montre|affiche|liste|dis)(?: moi)? (?:ce qu il y a|ce qui est (?:affiche|ouvert)|les {WINDOWS}))(?: d affiche| d ouvert)? sur {SCREEN}$"));
lazy_re!(background_re, format!(r"\b{WINDOWS}(?: qui (?:tournent|sont))? en (?:fond|arriere plan|tache de fond)\b"));
lazy_re!(browser_ask_re, format!("^(?:(?:dis|donne|montre|affiche|liste|lis)(?: moi)? )?(?:quelles? |quels? |c est quoi |combien (?:de |d )?)?(?:(?:sont |est )?(?:la |les |le )?)?(?:pages?|onglets?|sites?|instances?|fenetres?)(?: (?:sont|est))?(?: ouvertes?| ouverts?| affichees?| lancees?)?(?: (?:sur|dans|de)) {BROWSER}(?: en ce moment| actuellement)?$|^qu est ce qui est ouvert (?:sur|dans) {BROWSER}$|^(?:qu est ce que je regarde|sur quelle page (?:je suis|suis je))(?: (?:sur|dans) {BROWSER})?$"));
lazy_re!(minimize_all_re, format!("^(?:reduis|reduit|minimise|cache)(?: moi)? (?:tout|toutes les fenetres)(?: sur {SCREEN})?$"));
lazy_re!(move_re, format!("^(?:mets|met|deplace|bouge|envoie|passe|transfere|bascule)(?: moi)? (.+?) sur {SCREEN}$"));
lazy_re!(center_re, format!("^(?:centre|recentre|(?:mets|met|place|affiche|ramene)(?: moi)? (.+?) au (?:milieu|centre))(?: moi)?(?: (.+?))?(?: (?:de|sur) {SCREEN})?$"));
lazy_re!(show_re, format!("^{VERB_SHOW}(?: moi)? (.+)$"));
lazy_re!(page_activate_re, format!("^(?:va|passe|bascule|reviens|affiche|montre|ouvre|active)(?: moi)?(?: sur| a)? (?:la |l )?(?:page|onglet) {N}(?: point | virgule | ){N}$"));
lazy_re!(page_close_re, format!("^(?:ferme|fermer)(?: moi)? (?:la |l )?(?:page|onglet) {N}(?: point | virgule | ){N}$"));
lazy_re!(page_reload_re, format!("^(?:recharge|actualise|rafraichis)(?: moi)? (?:la |l )?(?:page|onglet) {N}(?: point | virgule | ){N}$"));
lazy_re!(page_read_re, format!("^(lis|lire|resume|resumer)(?: moi)? (?:la |l )?(?:page|onglet) {N}(?: point | virgule | ){N}$"));
lazy_re!(page_click_re, format!("^clique(?: moi)? sur (.+?) (?:dans|sur) (?:la |l )?(?:page|onglet) {N}(?: point | virgule | ){N}$"));

fn devant_re() -> &'static regex::Regex {
    crate::re!(r"^(?:(?:affiche|montre|ouvre|donne|mets)(?: moi)? )?(?:la |une )?(?:card |carte )?devant$|^applications? devant$|^applis? devant$|^quelles? (?:sont les )?(?:applis?|applications?) (?:qui sont )?devant$|^(?:toutes )?les applications? devant$|^(?:toutes )?les applis? devant$|^fenetres? devant$|^les fenetres? devant$")
}
fn foreground_re() -> &'static regex::Regex {
    crate::re!(r"^(?:quelle (?:est l )?(?:appli|application|fenetre) (?:est )?(?:au premier plan|active)|qu est ce qui est au premier plan|sur quelle (?:appli|application|fenetre) (?:je suis|suis je))$")
}

fn num(w: &str) -> i64 {
    if let Ok(n) = w.parse() {
        return n;
    }
    match w {
        "un" | "une" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "onze" => 11,
        "douze" => 12,
        "treize" => 13,
        "quatorze" => 14,
        "quinze" => 15,
        "seize" => 16,
        "vingt" => 20,
        _ => 0,
    }
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())).unwrap_or_else(|| url.to_string())
}

/// Boutons d'une fenêtre dans une card : ◎ premier plan, ⇄ autre écran, ▁ réduire, ✕ fermer.
pub fn window_actions(w: &J, monitor_count: usize) -> Vec<J> {
    let call = |a: &str| json!({ "method": "desktopAction", "args": [w["hwnd"], a] });
    let app = js::gs(w, "app");
    let m = w["monitor"].as_i64().unwrap_or(0);
    let other = if monitor_count == 2 { Some(if m == 1 { 2 } else { 1 }) } else { None };
    let mut v = vec![json!({ "icon": "◎", "title": format!("Mettre {app} au premier plan"), "call": call("afficher") })];
    if monitor_count > 1 {
        v.push(json!({ "icon": "⇄", "title": other.map(|o| format!("Déplacer sur l’écran {o}")).unwrap_or_else(|| "Changer d’écran".into()), "call": call("deplacer") }));
    }
    v.push(json!({ "icon": "▁", "title": "Réduire", "call": call("reduire") }));
    v.push(json!({ "icon": "✕", "tone": "danger", "title": format!("Fermer {app} (accord oral)"), "call": call("fermer") }));
    v
}

/// Card « Bureau » (ou « Écran 2 ») : fenêtres numérotées, groupées par écran, puis l'arrière-plan.
pub fn desktop_card(o: &Orchestrator, monitor: Option<i64>, compact: bool) -> J {
    let (ds, slots) = (&o.desktop_state, &o.window_slots);
    let mons = ds.monitors();
    let list = ds.windows_filtered(monitor, false);
    ds.remember_numbers(&list);
    // Emplacements : toutes les fenêtres comptent (une disposition peut couvrir plusieurs écrans).
    let marks = slots.marks(&ds.windows());
    let mut rows: Vec<J> = vec![json!({ "slotbar": slots.counts() })];
    let mut n = 0;
    for m in &mons {
        let mn = m["n"].as_i64().unwrap_or(0);
        if monitor.map(|x| x != mn).unwrap_or(false) {
            continue;
        }
        let here: Vec<&J> = list.iter().filter(|w| w["monitor"].as_i64() == Some(mn)).collect();
        rows.push(json!({ "section": format!("{} · {}×{}{}", js::gs(m, "label"), js::s(&m["rect"]["w"]), js::s(&m["rect"]["h"]), if here.is_empty() { " · aucune fenêtre" } else { "" }) }));
        for w in here {
            n += 1;
            let app = js::gs(w, "app");
            let fg = js::gb(w, "fg");
            let mut row = json!({
                "icon": if fg { "▶" } else if js::gb(w, "min") { "▁" } else { "▣" },
                "title": format!("{n}. {app}"),
                "sub": if js::truthy(&w["page"]) { w["page"].clone() } else { w["title"].clone() },
                "meta": w["state"],
                "hint": format!("Clic : afficher {app} · « la fenêtre {n} »"),
                "call": { "method": "desktopAction", "args": [w["hwnd"], "afficher"] },
                // Pas de bouton ◎ : un clic sur la ligne (le titre) met déjà la fenêtre au premier plan.
                "actions": window_actions(w, mons.len()).into_iter().filter(|a| a["call"]["args"][1] != "afficher").collect::<Vec<_>>(),
                "slots": w["hwnd"].as_i64().and_then(|h| marks.get(&h)).copied().unwrap_or([false; SLOTS]).to_vec(),
                "hwnd": w["hwnd"]
            });
            if fg {
                row["tone"] = json!("ok");
            }
            // Terminal : couleur de son onglet (ou de chacun de ses onglets), la même que dans Windows Terminal.
            let colors = w["hwnd"].as_i64().map(|h| o.terminal_colors.colors_of_window(h)).unwrap_or_default();
            if let Some(c) = colors.first() {
                row["accent"] = json!(c);
                if colors.len() > 1 {
                    row["accents"] = json!(colors);
                }
            }
            rows.push(row);
        }
    }
    let bg = if monitor.is_none() { ds.background() } else { vec![] };
    if !bg.is_empty() {
        let hid = &o.bg_hidden;
        let showing = hid.showing.get();
        let (hidden, shown): (Vec<&J>, Vec<&J>) = bg.iter().partition(|b| hid.is_hidden(&js::gs(b, "app")));
        rows.push(json!({ "section": format!("En arrière-plan · {}", shown.len()) }));
        for b in shown {
            let app = js::gs(b, "app");
            rows.push(json!({
                "icon": "◌", "title": app, "sub": "ouverte sans fenêtre",
                "actions": [{ "icon": "🙈", "title": format!("Masquer {app} de cette liste"), "call": { "method": "desktopAction", "args": [0, format!("bg_hide:{app}")] } }]
            }));
        }
        if !hidden.is_empty() {
            let n = hidden.len();
            rows.push(json!({
                "icon": "👁", "title": if showing { "Cacher les masquées".to_string() } else { format!("{n} masquée{}", if n > 1 { "s" } else { "" }) },
                "hint": "Afficher ou cacher les applications masquées",
                "call": { "method": "desktopAction", "args": [0, "bg_toggle"] }
            }));
            if showing {
                for b in hidden {
                    let app = js::gs(b, "app");
                    rows.push(json!({
                        "icon": "◌", "title": app, "sub": "masquée", "tone": "dim",
                        "actions": [{ "icon": "↺", "title": format!("Remettre {app} dans la liste"), "call": { "method": "desktopAction", "args": [0, format!("bg_show:{app}")] } }]
                    }));
                }
            }
        }
    }
    let title = match monitor {
        None => "Bureau".to_string(),
        Some(m) => crate::re!(r" \(principal\)$").replace(&ds.monitor_label(m), "").into_owned(),
    };
    let mut card = json!({
        "title": title, "tool": "bureau", "wide": true, "count": list.len(), "rows": rows, "ttl": 180000,
        "empty": "Aucune fenêtre ouverte.",
        "footer": "clic sur une fenêtre : premier plan · ⇄ autre écran · ✕ fermer · 1-4 emplacements (clic : y aller ou l’enregistrer · clic droit : réenregistrer · Maj+clic : effacer)"
    });
    if compact {
        card["compact"] = json!(true);
        card["pinned"] = json!(true);
    }
    card
}

/// Card d'un navigateur : instances numérotées, avec leurs pages « 1.1, 1.2… ».
pub fn browser_card(o: &Orchestrator, browser: &str) -> J {
    let ds = &o.desktop_state;
    let bridge = &o.browser_bridge;
    let key = if fold(browser) == "edge" { "msedge".to_string() } else { fold(browser) };
    let name = browser_of(&key).map(String::from).unwrap_or_else(|| browser.to_string());
    let mons = ds.monitors();
    let mut rows: Vec<J> = vec![];
    let mut pages = 0;
    let tab_row = |inst: i64, tab: &J, j: usize| {
        let active = js::gb(tab, "active");
        let audible = js::gb(tab, "audible");
        let meta: Vec<&str> = [if active { "affichée" } else { "" }, if audible { "🔊" } else { "" }, if js::gb(tab, "pinned") { "📌" } else { "" }].into_iter().filter(|s| !s.is_empty()).collect();
        let title = if js::truthy(&tab["title"]) { js::gs(tab, "title") } else if js::truthy(&tab["url"]) { js::gs(tab, "url") } else { "(sans titre)".into() };
        let mut r = json!({
            "icon": if active { "▶" } else if audible { "♪" } else { "◻" },
            "title": format!("{inst}.{} {title}", j + 1), "sub": host_of(&js::gs(tab, "url")), "meta": meta.join(" "),
            "hint": format!("Clic : aller sur cette page · « va sur la page {inst}.{} »", j + 1),
            "call": { "method": "browserTab", "args": [tab["id"], "activer"] },
            "actions": [
                { "icon": "⟳", "title": "Recharger", "call": { "method": "browserTab", "args": [tab["id"], "recharger"] } },
                { "icon": "✕", "tone": "danger", "title": "Fermer cet onglet", "call": { "method": "browserTab", "args": [tab["id"], "fermer"] }, "done": "fermé" }
            ]
        });
        if active {
            r["tone"] = json!("ok");
        }
        r
    };
    if key == "chrome" && bridge.connected() {
        let list = bridge.numbered(Some(ds));
        bridge.remember(&list);
        ds.remember_browser(&key, &list.iter().filter(|x| x["desktop"].is_object()).map(|x| x["desktop"].clone()).collect::<Vec<_>>());
        for x in &list {
            let w = &x["desktop"];
            let inst = x["instance"].as_i64().unwrap_or(0);
            let state = if w.is_object() { js::gs(w, "state") } else if js::gs(&x["chrome"], "state") == "minimized" { "réduite".into() } else { "ouverte".into() };
            let tabs = js::arr(&x["chrome"]["tabs"]);
            let screen = if w.is_object() { ds.monitor_label(w["monitor"].as_i64().unwrap_or(0)) } else { "écran ?".into() };
            rows.push(json!({ "section": format!("{name} {inst} · {screen} · {state} · {} onglet{}", tabs.len(), if tabs.len() > 1 { "s" } else { "" }) }));
            for (j, tab) in tabs.iter().enumerate() {
                pages += 1;
                rows.push(tab_row(inst, tab, j));
            }
        }
        return json!({
            "title": name, "tool": "navigateur", "wide": true, "count": list.len(), "rows": rows, "ttl": 180000, "pages": pages,
            "empty": format!("{name} n’est pas ouvert."),
            "footer": "« va sur la page 2.3 » · « ferme la page 1.2 » · « ferme les onglets YouTube » · « ouvre … dans un nouvel onglet »"
        });
    }
    let list = ds.browser_instances(&key);
    ds.remember_browser(&key, &list);
    for w in &list {
        let inst = js::s(&w["instance"]);
        rows.push(json!({ "section": format!("{name} {inst} · {} · {}", ds.monitor_label(w["monitor"].as_i64().unwrap_or(0)), js::gs(w, "state")) }));
        pages += 1;
        let mut wa = w.clone();
        wa["app"] = json!(format!("{name} {inst}"));
        let mut r = json!({
            "icon": "▶", "title": format!("{inst}.1 {}", if js::truthy(&w["page"]) { js::gs(w, "page") } else { "(sans titre)".into() }),
            "sub": "page affichée", "hint": format!("Clic : afficher {name} {inst}"),
            "call": { "method": "desktopAction", "args": [w["hwnd"], "afficher"] }, "actions": window_actions(&wa, mons.len())
        });
        if js::gb(w, "fg") {
            r["tone"] = json!("ok");
        }
        rows.push(r);
    }
    json!({
        "title": name, "tool": "navigateur", "wide": true, "count": list.len(), "rows": rows, "ttl": 180000, "pages": pages,
        "empty": format!("{name} n’est pas ouvert."),
        "footer": if key == "chrome" { format!("« affiche {name} 2 » · tous les onglets de chaque instance : installe l’extension Jarvis (« relie Chrome à Jarvis »)") } else { format!("« affiche {name} 2 » · « ferme {name} 2 »") }
    })
}

fn failed(r: &J) -> J {
    let e = if js::truthy(&r["error"]) { js::gs(r, "error") } else { "échec".into() };
    json!({ "say": if js::gb(r, "denied") { "Action refusée.".to_string() } else { format!("Je n'ai pas pu : {e}.") }, "display": "Échec" })
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

/// La card affichée est suivie : l'orchestrateur la remet à jour en direct.
fn show(o: &Orchestrator, card: &J, monitor: Option<i64>) {
    o.show_widget("tool", card.clone());
    let last = if js::gs(card, "tool") == "navigateur" {
        json!({ "kind": "navigateur", "browser": card["title"], "title": card["title"] })
    } else {
        json!({ "kind": "bureau", "monitor": monitor, "title": card["title"] })
    };
    *o.last_desktop_card.borrow_mut() = Some(last);
    *o.desktop_card_sig.borrow_mut() = card["rows"].to_string();
}

/// Action sur une fenêtre derrière local_tool.
async fn act(o: &Orchestrator, action: &str, w: Option<&J>, extra: J) -> J {
    let name = if action == "fermer" { "mcp__system__fenetre_fermer" } else { "mcp__system__fenetre" };
    let mut params = json!({ "action": action, "cible": w.map(|w| w["app"].clone()).unwrap_or(json!("toutes")) });
    js::assign(&mut params, extra.clone());
    let mut exec = json!({ "action": action, "hwnd": w.map(|w| w["hwnd"].clone()).unwrap_or(Value::Null) });
    js::assign(&mut exec, extra);
    let oc = o.self_ref();
    o.local_tool(name, params, Box::new(move |_| Box::pin(async move { oc.up().desktop_state.tool_action(&exec).await }))).await
}

pub fn desktop_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    let orch = o.up();
    let ds = &orch.desktop_state;
    if !ds.ready() {
        return None;
    }

    // --- Lecture ---------------------------------------------------------------------------------
    if let Some(m) = on_screen_re().captures(t) {
        let spoken = m[1].to_string();
        let monitor = ds.resolve_monitor(&spoken);
        return Some(it(o, "desktop_screen", "Bureau", &spoken, move |o| async move {
            let ds = &o.desktop_state;
            let Some(monitor) = monitor else { return json!({ "say": format!("Je ne trouve pas cet écran. Tu en as {}.", ds.monitors().len()), "display": "Écran inconnu" }) };
            let card = desktop_card(&o, Some(monitor), false);
            show(&o, &card, Some(monitor));
            let list = ds.windows_filtered(Some(monitor), false);
            let mut names: Vec<String> = vec![];
            for w in &list {
                let a = js::gs(w, "app");
                if !names.contains(&a) {
                    names.push(a);
                }
            }
            let label = crate::re!(r" \(principal\)$").replace(&ds.monitor_label(monitor).to_lowercase(), " principal").into_owned();
            json!({
                "say": if list.is_empty() { format!("Aucune fenêtre sur l'écran {monitor}.") } else { format!("Sur l'{label} : {}.", names.into_iter().take(6).collect::<Vec<_>>().join(", ")) },
                "display": format!("{} fenêtres", list.len())
            })
        }));
    }
    if devant_re().is_match(t) {
        return Some(it(o, "desktop_devant", "Devant", "applications en cours", |o| async move {
            let card = o.show_devant_card();
            let count = js::arr(&card["items"]).len();
            json!({ "say": if count > 0 { format!("{count} application{} en cours.", if count > 1 { "s" } else { "" }) } else { "Aucune application ouverte.".into() }, "display": format!("Devant · {count} applis") })
        }));
    }
    let only_bg = background_re().is_match(t);
    if list_apps_re().is_match(t) || only_bg {
        return Some(it(o, "desktop_list", "Bureau", if only_bg { "arrière-plan" } else { "fenêtres" }, move |o| async move {
            let ds = &o.desktop_state;
            show(&o, &desktop_card(&o, None, false), None);
            let list = ds.windows();
            let bg = ds.background();
            if only_bg {
                return json!({
                    "say": if bg.is_empty() { "Aucune application n'est ouverte en arrière-plan.".to_string() } else { format!("En arrière-plan : {}.", bg.iter().take(8).map(|b| js::gs(b, "app")).collect::<Vec<_>>().join(", ")) },
                    "display": format!("{} en fond", bg.len())
                });
            }
            let mut names: Vec<String> = vec![];
            for w in &list {
                let a = js::gs(w, "app");
                if !names.contains(&a) {
                    names.push(a);
                }
            }
            let n = list.len();
            json!({
                "say": format!(
                    "{n} fenêtre{} : {}{}. Le détail par écran est à l'écran.",
                    if n > 1 { "s" } else { "" },
                    names.into_iter().take(8).collect::<Vec<_>>().join(", "),
                    if bg.is_empty() { String::new() } else { format!(", et {} appli{} en arrière-plan", bg.len(), if bg.len() > 1 { "s" } else { "" }) }
                ),
                "display": format!("{n} fenêtres")
            })
        }));
    }
    if foreground_re().is_match(t) {
        return Some(it(o, "desktop_foreground", "Bureau", "premier plan", |o| async move {
            match o.desktop_state.windows().into_iter().find(|w| js::gb(w, "fg")) {
                Some(w) => json!({
                    "say": format!("{}{}, sur l'écran {}.", js::gs(&w, "app"), if js::truthy(&w["page"]) { format!(", sur la page {}", js::gs(&w, "page")) } else { String::new() }, js::s(&w["monitor"])),
                    "display": w["app"]
                }),
                None => json!({ "say": "Aucune fenêtre au premier plan.", "display": "—" }),
            }
        }));
    }
    if let Some(m) = browser_ask_re().captures(t) {
        let mut b = m.iter().skip(1).flatten().next().map(|x| x.as_str().to_string()).unwrap_or_else(|| "chrome".into());
        if b.contains("navigateur") {
            b = BROWSER_KEYS.iter().find(|k| !ds.browser_instances(k).is_empty()).map(|k| k.to_string()).unwrap_or_else(|| "chrome".into());
        }
        let b2 = b.clone();
        return Some(it(o, "desktop_browser", "Navigateur", &b, move |o| {
            let b = b2.clone();
            async move {
                let card = browser_card(&o, &b);
                show(&o, &card, None);
                let name = js::gs(&card, "title");
                let ds = &o.desktop_state;
                if fold(&b) == "chrome" && o.browser_bridge.connected() {
                    let list = o.browser_bridge.numbered(Some(ds));
                    if list.is_empty() {
                        return json!({ "say": format!("{name} n'est pas ouvert."), "display": format!("{name} fermé") });
                    }
                    let tabs: usize = list.iter().map(|x| js::arr(&x["chrome"]["tabs"]).len()).sum();
                    let shown: Vec<String> = list
                        .iter()
                        .map(|x| {
                            let ts = js::arr(&x["chrome"]["tabs"]);
                            let a = ts.iter().find(|t| js::gb(t, "active"));
                            format!(
                                "{name} {}{}, {} onglet{}{}",
                                js::s(&x["instance"]),
                                if x["desktop"].is_object() { format!(" sur l'écran {}", js::s(&x["desktop"]["monitor"])) } else { String::new() },
                                ts.len(),
                                if ts.len() > 1 { "s" } else { "" },
                                a.map(|a| format!(", page affichée : {}", js::gs(a, "title"))).unwrap_or_default()
                            )
                        })
                        .collect();
                    let head = if list.len() > 1 { format!("{} instances de {name}", list.len()) } else { format!("Une instance de {name}") };
                    return json!({ "say": format!("{head}, {tabs} onglet{} en tout. {}. La liste numérotée est à l'écran.", if tabs > 1 { "s" } else { "" }, shown.join(" ; ")), "display": format!("{tabs} onglets") });
                }
                let key = if fold(&b) == "edge" { "msedge".to_string() } else { fold(&b) };
                let list = ds.browser_instances(&key);
                let page = |w: &J| if js::truthy(&w["page"]) { js::gs(w, "page") } else { "sans titre".into() };
                match list.len() {
                    0 => json!({ "say": format!("{name} n'est pas ouvert."), "display": format!("{name} fermé") }),
                    1 => json!({ "say": format!("Une instance de {name}, sur l'écran {}, sur la page {}.", js::s(&list[0]["monitor"]), page(&list[0])), "display": name }),
                    n => json!({
                        "say": format!("{n} instances de {name} : {}.", list.iter().map(|w| format!("{name} {} sur l'écran {}, page {}", js::s(&w["instance"]), js::s(&w["monitor"]), page(w))).collect::<Vec<_>>().join(" ; ")),
                        "display": format!("{n} instances")
                    }),
                }
            }
        }));
    }

    if let Some(i) = chrome_tab_intents(o, t, raw) {
        return Some(i);
    }
    if let Some(i) = ui_intents(o, t, raw) {
        return Some(i);
    }

    // --- Actions ---------------------------------------------------------------------------------
    if let Some(m) = minimize_all_re().captures(t) {
        let screen = m.get(1).map(|x| x.as_str().to_string());
        let detail = screen.clone().unwrap_or_else(|| "tout".into());
        return Some(it(o, "desktop_minimize_all", "Bureau", &detail, move |o| {
            let screen = screen.clone();
            async move {
                let r = act(&o, "tout_reduire", None, screen.map(|s| json!({ "ecran": s })).unwrap_or(json!({}))).await;
                if js::gb(&r, "ok") { json!({ "say": format!("{}.", js::gs(&r, "message")), "display": r["message"] }) } else { failed(&r) }
            }
        }));
    }
    if let Some(m) = move_re().captures(t) {
        if let Some(w) = ds.find_window(&m[1], false) {
            let screen = m[2].to_string();
            let app = js::gs(&w, "app");
            return Some(it(o, "desktop_move", "Bureau", &format!("{app} → {screen}"), move |o| {
                let (w, screen) = (w.clone(), screen.clone());
                async move {
                    let extra = if screen.contains("autre ecran") { json!({}) } else { json!({ "ecran": screen }) };
                    let r = act(&o, "deplacer", Some(&w), extra).await;
                    let app = js::gs(&w, "app");
                    if js::gb(&r, "ok") { json!({ "say": format!("{app} est sur l'écran {}.", js::s(&r["ecran"])), "display": format!("{app} · écran {}", js::s(&r["ecran"])) }) } else { failed(&r) }
                }
            }));
        }
    }
    if let Some(m) = center_re().captures(t) {
        let target = m.get(1).or(m.get(2)).map(|x| x.as_str().to_string());
        if let Some(w) = target.and_then(|tg| ds.find_window(&tg, false)) {
            let screen = m.get(3).map(|x| x.as_str().to_string());
            let app = js::gs(&w, "app");
            return Some(it(o, "desktop_center", "Bureau", &format!("{app} au milieu"), move |o| {
                let (w, screen) = (w.clone(), screen.clone());
                async move {
                    let r = act(&o, "centrer", Some(&w), screen.map(|s| json!({ "ecran": s })).unwrap_or(json!({}))).await;
                    let app = js::gs(&w, "app");
                    if js::gb(&r, "ok") { json!({ "say": format!("{app} au milieu de l'écran {}.", js::s(&r["ecran"])), "display": format!("{app} · écran {}", js::s(&r["ecran"])) }) } else { failed(&r) }
                }
            }));
        }
    }
    if let Some(m) = crate::re!(r"^(?:mets|met|place|colle|range)(?: moi)? (.+?) (?:a|sur la moitie|sur la partie|du cote) (gauche|droite)(?: de l ecran)?$").captures(t) {
        if let Some(w) = ds.find_window(&m[1], false) {
            let side = m[2].to_string();
            let app = js::gs(&w, "app");
            return Some(it(o, "desktop_snap", "Bureau", &format!("{app} à {side}"), move |o| {
                let (w, side) = (w.clone(), side.clone());
                async move {
                    let r = act(&o, &side, Some(&w), json!({})).await;
                    let app = js::gs(&w, "app");
                    if js::gb(&r, "ok") { json!({ "say": format!("{app} à {side}."), "display": format!("{app} · {side}") }) } else { failed(&r) }
                }
            }));
        }
    }
    let shape = if let Some(m) = crate::re!(r"^(?:reduis|reduit|minimise)(?: moi)? (.+)$").captures(t) {
        Some(("reduire", m[1].to_string()))
    } else if let Some(m) = crate::re!(r"^(?:agrandis|agrandit|maximise|mets en plein ecran)(?: moi)? (.+)$").captures(t) {
        Some(("agrandir", m[1].to_string()))
    } else {
        show_re().captures(t).map(|m| ("afficher", m[1].to_string()))
    };
    if let Some((action, spoken)) = shape {
        if let Some(w) = ds.find_window(&spoken, false) {
            let verb = match action {
                "reduire" => "réduite",
                "agrandir" => "agrandie",
                _ => "au premier plan",
            };
            let app = js::gs(&w, "app");
            return Some(it(o, &format!("desktop_{action}"), "Bureau", &app, move |o| {
                let w = w.clone();
                async move {
                    let r = act(&o, action, Some(&w), json!({})).await;
                    let app = js::gs(&w, "app");
                    if js::gb(&r, "ok") { json!({ "say": if action == "afficher" { Value::Null } else { json!(format!("{app} {verb}.")) }, "display": format!("{app} · {verb}") }) } else { failed(&r) }
                }
            }));
        }
    }
    let close = crate::re!(r"^(?:ferme|fermer)(?: moi)? (?:la )?(fenetre (?:\d{1,2}|numero \d{1,2}))$").captures(t).or_else(|| crate::re!(r"^(?:ferme|fermer)(?: moi)? ((?:chrome|edge|brave|firefox|opera) (?:\d{1,2}|un|deux|trois|quatre))$").captures(t));
    if let Some(c) = close {
        let spoken = c[1].replace("numero ", "");
        let spoken = crate::re!(r" (un|deux|trois|quatre)$").replace(&spoken, |x: &regex::Captures| format!(" {}", num(&x[1]))).into_owned();
        if let Some(w) = ds.find_window(&spoken, true) {
            let app = js::gs(&w, "app");
            return Some(it(o, "desktop_close", "Bureau", &app, move |o| {
                let w = w.clone();
                async move {
                    let r = act(&o, "fermer", Some(&w), json!({})).await;
                    if !js::gb(&r, "ok") {
                        return failed(&r);
                    }
                    let app = js::gs(&w, "app");
                    let still = wait_gone(&o.desktop_state, &[w], 3000).await;
                    if still.is_empty() { json!({ "say": format!("Fenêtre de {app} fermée."), "display": format!("{app} fermée") }) } else { json!({ "say": format!("{app} ne s'est pas fermé : il attend peut-être une confirmation à l'écran."), "display": "Pas fermé" }) }
                }
            }));
        }
    }

    // Fermer une application : ses fenêtres, ou son processus si elle tourne sans fenêtre.
    if let Some(m) = crate::re!(r"^(?:ferme|fermer|quitte|quitter)(?: moi)?(?: l application| l appli| le logiciel| le programme| les fenetres (?:de |d )| la fenetre (?:de |d ))? (.+)$").captures(t) {
        let spoken = m[1].to_string();
        if !crate::re!(r"^(?:la |le |les |l )?(?:card|carte|cartes|cards|fiche|widget|note|dictee|session|volet|tout|tous|toutes|ca|cette|ce)\b").is_match(&spoken) {
            let wins = ds.match_windows(&spoken);
            let bg = if wins.is_empty() { ds.match_background(&spoken) } else { vec![] };
            let known = orch.app_index.find(&spoken).is_some();
            if !wins.is_empty() || !bg.is_empty() || known {
                let said = crate::re!(r"(?i)(?:ferme[rz]?|quitte[rz]?)\s+(?:l['’]\s*(?:application|appli)\s+|le\s+(?:logiciel|programme)\s+|les?\s+fen[êe]tres?\s+(?:de\s+|d['’]\s*)?)?(.+?)[\s.!?]*$").captures(raw).map(|c| c[1].to_string()).unwrap_or_else(|| spoken.clone());
                let by_name = !wins.is_empty() && wins.iter().all(|w| fold(&js::gs(w, "app")) == fold(&spoken) || fold(&js::gs(w, "process")) == fold(&spoken));
                let label = if !wins.is_empty() {
                    if by_name { js::gs(&wins[0], "app") } else { js::capitalize(&said) }
                } else if !bg.is_empty() {
                    super::desktop_state::app_name(&js::gs(&bg[0], "process"))
                } else {
                    js::capitalize(&said)
                };
                return Some(it(o, "app_close", "Fermeture", &label.clone(), move |o| {
                    let (wins, bg, label) = (wins.clone(), bg.clone(), label.clone());
                    async move {
                        if wins.is_empty() && bg.is_empty() {
                            return json!({ "say": format!("{label} n'est pas ouvert."), "display": format!("{label} fermé") });
                        }
                        let oc = o.self_ref();
                        let (w2, b2) = (wins.clone(), bg.clone());
                        let r = o
                            .local_tool("mcp__system__fermer_application", json!({ "nom": label }), Box::new(move |_| -> Fut<J> {
                                Box::pin(async move {
                                    let orch = oc.up();
                                    if !w2.is_empty() {
                                        let (closed, remaining) = orch.desktop_state.close_windows(&w2, 3000).await;
                                        return if remaining.is_empty() { json!({ "ok": true, "fermees": closed.len() }) } else { json!({ "ok": false, "restantes": remaining.len(), "error": "fenêtre toujours ouverte" }) };
                                    }
                                    match crate::orch::system::call("fermer_application", &json!({ "nom": b2[0]["process"] })).await {
                                        Some(Ok(v)) => v,
                                        Some(Err(e)) => json!({ "ok": false, "error": e }),
                                        None => json!({ "ok": false, "error": "action indisponible" }),
                                    }
                                })
                            }))
                            .await;
                        if js::gb(&r, "denied") {
                            return json!({ "say": "Fermeture refusée.", "display": "Refusé" });
                        }
                        if js::gb(&r, "ok") {
                            let psmux = wins.iter().any(|w| crate::re!(r"(?i)tmux|psmux").is_match(&js::gs(w, "title")));
                            return json!({
                                "say": format!("{label}{} est fermé.{}", if wins.len() > 1 { format!(" ({} fenêtres)", wins.len()) } else { String::new() }, if psmux { " La session psmux continue en arrière-plan ; dis « ferme la session tmux » pour l’arrêter aussi." } else { "" }),
                                "display": format!("{label} fermé")
                            });
                        }
                        if js::truthy(&r["restantes"]) {
                            return json!({ "say": format!("{label} ne s'est pas fermé : il attend peut-être une confirmation à l'écran."), "display": "Pas fermé" });
                        }
                        json!({ "say": format!("Je n'ai pas pu fermer {label}{}.", if js::truthy(&r["error"]) { format!(" : {}", js::gs(&r, "error")) } else { String::new() }), "display": "Échec" })
                    }
                }));
            }
        }
    }

    // Ouvrir une application déjà ouverte : elle est affichée au lieu d'être relancée.
    if let Some(m) = crate::re!(r"^(?:ouvre|lance|demarre)(?: moi)? (?:l application |l appli |le logiciel |le programme )?(.+)$").captures(t) {
        if !crate::re!(r"^(?:un |une |le dossier|la page|l onglet|un nouvel|une nouvelle|nouvel|nouvelle|(?:la |le |les )?(?:card|cards|carte|cartes|carde|cardes|module|modules|widget|widgets|fiche|fiches)\b)").is_match(&m[1]) {
            if let Some(w) = ds.find_window(&m[1], false) {
                let app = js::gs(&w, "app");
                return Some(it(o, "app_show_open", "Application", &app, move |o| {
                    let w = w.clone();
                    async move {
                        let r = act(&o, "afficher", Some(&w), json!({})).await;
                        let app = js::gs(&w, "app");
                        if js::gb(&r, "ok") {
                            let many = o.desktop_state.monitors().len() > 1;
                            json!({ "say": format!("{app} est déjà ouvert, je l'affiche{}.", if many { format!(" sur l'écran {}", js::s(&w["monitor"])) } else { String::new() }), "display": format!("{app} · affiché") })
                        } else {
                            failed(&r)
                        }
                    }
                }));
            }
        }
    }
    None
}

async fn tabs_tool(o: &Orchestrator, tool: &str, params: J) -> J {
    let oc = o.self_ref();
    o.local_tool(tool, params, Box::new(move |p| Box::pin(async move {
        let orch = oc.up();
        orch.browser_bridge.tool_action(&p, Some(&orch.desktop_state)).await
    })))
    .await
}

fn need_bridge() -> J {
    json!({ "say": "L'extension Jarvis n'est pas reliée à Chrome. Dis « relie Chrome à Jarvis » pour l'installer.", "display": "Extension absente" })
}

fn refresh_browser_card(o: &Orchestrator) {
    let navigating = o.last_desktop_card.borrow().as_ref().map(|c| js::gs(c, "kind") == "navigateur").unwrap_or(false);
    if navigating {
        let oc = o.self_ref();
        js::set_timeout(400, move || oc.up().refresh_desktop_card());
    }
}

/// Commandes sur les onglets de Chrome (extension Jarvis).
fn chrome_tab_intents(o: &OrchRef, t: &str, raw: &str) -> Option<Intent> {
    let ref_of = |m: &regex::Captures, i: usize| format!("{}.{}", num(&m[i]), num(&m[i + 1]));

    if crate::re!(r"^(?:relie|connecte|installe|appaire|active)(?: moi)? (?:chrome|l extension(?: jarvis)?(?: chrome| de chrome| pour chrome)?)(?: (?:a|avec) jarvis| (?:a|avec) toi)?$").is_match(t)
        || crate::re!(r"^(?:comment )?(?:installer|relier|connecter) (?:l extension|chrome)(?: a jarvis)?$").is_match(t)
    {
        return Some(it(o, "browser_pair", "Extension Chrome", "installation", |o| async move {
            let dir = crate::paths::root().join("browser-extension");
            let c = o.browser_bridge.connected();
            o.show_widget(
                "tool",
                json!({
                    "title": "Extension Chrome", "tool": "navigateur", "wide": true, "ttl": 300000,
                    "rows": [
                        { "icon": if c { "✓" } else { "1" }, "tone": if c { json!("ok") } else { Value::Null }, "title": if c { "Déjà reliée à Jarvis" } else { "Ouvre chrome://extensions dans Chrome" }, "sub": if c { "rien à faire" } else { "tape l’adresse dans la barre de Chrome" } },
                        { "icon": "2", "title": "Active le « Mode développeur »", "sub": "interrupteur en haut à droite" },
                        { "icon": "3", "title": "Clique « Charger l’extension non empaquetée »", "sub": dir.to_string_lossy() },
                        { "icon": "4", "title": "C’est tout : l’appairage est automatique", "sub": "Jarvis n’accepte que cette extension (identifiant fixe)" }
                    ],
                    "footer": "ensuite : « quelles pages sont ouvertes sur Chrome », « va sur la page 2.3 »"
                }),
            );
            if c {
                json!({ "say": "L'extension est déjà reliée : je vois tes onglets.", "display": "Extension reliée" })
            } else {
                json!({ "say": "Voici les étapes à l'écran : ouvre chrome deux points extensions, active le mode développeur, puis charge le dossier browser-extension de Jarvis. L'appairage est automatique.", "display": "Installation de l’extension" })
            }
        }));
    }
    if let Some(m) = page_activate_re().captures(t) {
        let r = ref_of(&m, 1);
        return Some(it(o, "tab_activate", "Onglet", &r.clone(), move |o| {
            let r = r.clone();
            async move {
                if !o.browser_bridge.connected() {
                    return need_bridge();
                }
                let x = tabs_tool(&o, "mcp__system__onglets", json!({ "action": "activer", "page": r })).await;
                if js::gb(&x, "ok") { json!({ "say": null, "display": if js::truthy(&x["titre"]) { js::gs(&x, "titre") } else { r } }) } else { failed(&x) }
            }
        }));
    }
    if let Some(m) = page_close_re().captures(t) {
        let r = ref_of(&m, 1);
        return Some(it(o, "tab_close", "Onglet", &r.clone(), move |o| {
            let r = r.clone();
            async move {
                if !o.browser_bridge.connected() {
                    return need_bridge();
                }
                let x = tabs_tool(&o, "mcp__system__onglets", json!({ "action": "fermer", "page": r })).await;
                refresh_browser_card(&o);
                if js::gb(&x, "ok") { json!({ "say": format!("Onglet fermé : {}.", js::s(&x["titres"][0])), "display": "Onglet fermé" }) } else { failed(&x) }
            }
        }));
    }
    if let Some(m) = page_reload_re().captures(t) {
        let r = ref_of(&m, 1);
        return Some(it(o, "tab_reload", "Onglet", &r.clone(), move |o| {
            let r = r.clone();
            async move {
                if !o.browser_bridge.connected() {
                    return need_bridge();
                }
                let x = tabs_tool(&o, "mcp__system__onglets", json!({ "action": "recharger", "page": r })).await;
                if js::gb(&x, "ok") { json!({ "say": null, "display": "Page rechargée" }) } else { failed(&x) }
            }
        }));
    }
    if let Some(m) = page_read_re().captures(t) {
        let r = ref_of(&m, 2);
        let summary = m[1].starts_with("resum");
        return Some(it(o, if summary { "tab_summary" } else { "tab_read" }, "Onglet", &r.clone(), move |o| {
            let r = r.clone();
            async move {
                if !o.browser_bridge.connected() {
                    return need_bridge();
                }
                let x = tabs_tool(&o, "mcp__system__onglets", json!({ "action": "lire", "page": r })).await;
                if !js::gb(&x, "ok") {
                    return failed(&x);
                }
                if summary {
                    o.enqueue_ai(
                        format!("Résume cette page web en trois ou quatre phrases, en français, pour me le dire à voix haute :\n\nTitre : {}\nAdresse : {}\n\n{}", js::gs(&x, "title"), js::gs(&x, "url"), js::slice_chars(&js::gs(&x, "text"), 12000)),
                        json!({ "priority": 1 }),
                    );
                    return json!({ "say": null, "display": "Résumé demandé" });
                }
                let clean = crate::re!(r"\s+").replace_all(&js::gs(&x, "text"), " ").trim().to_string();
                let cut = if clean.chars().count() > 700 { format!("{}…", crate::re!(r"\s\S*$").replace(&js::slice_chars(&clean, 700), "")) } else { clean };
                json!({ "say": format!("{}. {cut}", js::gs(&x, "title")), "display": x["title"] })
            }
        }));
    }
    if let Some(m) = crate::re!(r"^(?:ferme|fermer)(?: moi)? (?:les |tous les )?onglets? (?:de |du |des |d |sur |avec )?(.+)$").captures(t) {
        if !m[1].starts_with(|c: char| c.is_ascii_digit()) {
            let what = m[1].to_string();
            return Some(it(o, "tabs_close_match", "Onglets", &what.clone(), move |o| {
                let what = what.clone();
                async move {
                    if !o.browser_bridge.connected() {
                        return need_bridge();
                    }
                    if o.browser_bridge.find_tabs(&what, Some(&o.desktop_state)).is_empty() {
                        return json!({ "say": format!("Aucun onglet ne correspond à {what}."), "display": "Aucun onglet" });
                    }
                    let x = tabs_tool(&o, "mcp__system__onglets", json!({ "action": "fermer", "page": what })).await;
                    refresh_browser_card(&o);
                    let n = x["fermees"].as_i64().unwrap_or(0);
                    let s = if n > 1 { "s" } else { "" };
                    if js::gb(&x, "ok") { json!({ "say": format!("{n} onglet{s} fermé{s}."), "display": format!("{n} fermés") }) } else { failed(&x) }
                }
            }));
        }
    }
    if let Some(m) = crate::re!(r"^(?:va|passe|bascule|reviens)(?: moi)? (?:sur |a )?(?:la page|l onglet) (?:de |du |des |d )?(.+)$").captures(t) {
        let what = m[1].to_string();
        let orch = o.up();
        if !what.starts_with(|c: char| c.is_ascii_digit()) && orch.browser_bridge.connected() && !orch.browser_bridge.find_tabs(&what, Some(&orch.desktop_state)).is_empty() {
            return Some(it(o, "tab_activate", "Onglet", &what.clone(), move |o| {
                let what = what.clone();
                async move {
                    let x = tabs_tool(&o, "mcp__system__onglets", json!({ "action": "activer", "page": what })).await;
                    if js::gb(&x, "ok") { json!({ "say": null, "display": if js::truthy(&x["titre"]) { js::gs(&x, "titre") } else { what } }) } else { failed(&x) }
                }
            }));
        }
    }
    let new_tab = crate::re!(r"^(?:ouvre|affiche|cherche)(?: moi)? (.+?) dans un nouvel onglet$").captures(t).or_else(|| crate::re!(r"^(?:ouvre )?(?:un )?nouvel onglet(?: (?:sur|avec|pour) (.+))?$").captures(t));
    if let Some(m) = new_tab {
        let r = crate::re!(r"(?i)^jarvis[,\s]+").replace(raw, "").into_owned();
        let said = crate::re!(r"(?i)(?:ouvre|affiche|cherche)(?:-moi| moi)?\s+(.+?)\s+dans un nouvel onglet")
            .captures(&r)
            .or_else(|| crate::re!(r"(?i)nouvel onglet\s+(?:sur|avec|pour)\s+(.+?)[.!?]*$").captures(&r))
            .map(|c| c[1].to_string())
            .or_else(|| m.get(1).map(|x| x.as_str().to_string()))
            .unwrap_or_default();
        let detail = if said.is_empty() { "nouvel onglet".to_string() } else { said.clone() };
        return Some(it(o, "tab_open", "Onglet", &detail, move |o| {
            let said = said.clone();
            async move {
                if !o.browser_bridge.connected() {
                    return need_bridge();
                }
                let url = if said.is_empty() { "https://www.google.com".to_string() } else { said.clone() };
                let x = tabs_tool(&o, "mcp__system__onglets", json!({ "action": "ouvrir", "url": url })).await;
                if js::gb(&x, "ok") { json!({ "say": null, "display": if said.is_empty() { "Nouvel onglet".into() } else { said } }) } else { failed(&x) }
            }
        }));
    }
    if let Some(m) = page_click_re().captures(t) {
        let r = ref_of(&m, 2);
        let said = crate::re!(r"(?i)clique(?:-moi| moi)?\s+sur\s+(.+?)\s+(?:dans|sur)\s+(?:la |l['’])?(?:page|onglet)").captures(raw).map(|c| c[1].to_string()).unwrap_or_else(|| m[1].to_string());
        return Some(it(o, "tab_click", "Onglet", &format!("{said} ({r})"), move |o| {
            let (r, said) = (r.clone(), said.clone());
            async move {
                if !o.browser_bridge.connected() {
                    return need_bridge();
                }
                let texte = crate::re!(r#"^[«"“]\s*|\s*[»"”]$"#).replace_all(&said, "").into_owned();
                let x = tabs_tool(&o, "mcp__system__onglet_agir", json!({ "action": "cliquer", "page": r, "texte": texte })).await;
                if js::gb(&x, "ok") { json!({ "say": format!("J'ai cliqué sur {}.", if js::truthy(&x["clicked"]) { js::gs(&x, "clicked") } else { said }), "display": "Clic fait" }) } else { failed(&x) }
            }
        }));
    }
    None
}

fn unquote(s: &str) -> String {
    crate::re!(r#"^[«"“]\s*|\s*[»"”]$"#).replace_all(s.trim(), "").into_owned()
}

async fn agir(o: &Orchestrator, w: &J, params: J, exec: Box<dyn FnOnce(Rc<Orchestrator>) -> Fut<J>>) -> J {
    let mut p = json!({ "application": w["app"], "hwnd": w["hwnd"] });
    js::assign(&mut p, params);
    let oc = o.self_ref();
    o.local_tool("mcp__system__ui_agir", p, Box::new(move |_| exec(oc.up()))).await
}

fn ui_outcome(o: &Orchestrator, w: &J, r: &J, ok_say: String) -> J {
    if js::gb(r, "denied") {
        let app = js::gs(w, "app");
        return json!({ "say": if super::ui_control::UiControl::forbidden(Some(w)) { format!("Pour ta sécurité, je ne clique pas dans {app}.") } else { "Clic refusé.".into() }, "display": "Refusé" });
    }
    let _ = o;
    if js::gb(r, "ok") { json!({ "say": ok_say, "display": "Fait" }) } else { json!({ "say": format!("Je n'ai pas pu : {}.", if js::truthy(&r["error"]) { js::gs(r, "error") } else { "échec".into() }), "display": "Échec" }) }
}

/// Clics et saisies dans une application. Outil mcp__system__ui_agir : accord oral, sauf applis de confiance.
fn ui_intents(o: &OrchRef, t: &str, raw_text: &str) -> Option<Intent> {
    let orch = o.up();
    let raw = crate::re!(r"(?i)^jarvis[,\s]+").replace(raw_text.trim(), "");
    let raw = crate::re!(r"[.!?]+$").replace(&raw, "").into_owned();

    let trust = crate::re!(r"(?i)^(?:fais confiance à|fais confiance a)\s+(?:l['’]application\s+)?(.+?)(?:\s+pour les clics)?$").captures(&raw).or_else(|| crate::re!(r"(?i)^ajoute\s+(.+?)\s+aux\s+(?:applis|applications)\s+de confiance$").captures(&raw));
    if let Some(m) = trust {
        let name = unquote(&m[1]);
        return Some(it(o, "ui_trust", "Applis de confiance", &name.clone(), move |o| {
            let name = name.clone();
            async move {
                let w = if o.desktop_state.ready() { o.desktop_state.find_window(&name, false) } else { None };
                let label = w.as_ref().map(|w| js::gs(w, "app")).filter(|a| !a.is_empty()).unwrap_or(name);
                if super::ui_control::UiControl::forbidden(w.as_ref()) || forbidden_name(&label) {
                    return json!({ "say": format!("{label} reste protégée : pas de clics automatiques dans cette application."), "display": "Refusé" });
                }
                let added = o.ui_control.trust(&label);
                o.record("ui_confiance", json!({ "application": label, "action": "ajout" }));
                json!({ "say": if added { format!("D'accord : je cliquerai dans {label} sans te demander.") } else { format!("{label} est déjà une appli de confiance.") }, "display": label })
            }
        }));
    }
    let untrust = crate::re!(r"(?i)^ne fais plus confiance (?:à|a)\s+(.+)$").captures(&raw).or_else(|| crate::re!(r"(?i)^(?:retire|enlève|enleve)\s+(.+?)\s+des\s+(?:applis|applications)\s+de confiance$").captures(&raw));
    if let Some(m) = untrust {
        let name = unquote(&m[1]);
        return Some(it(o, "ui_untrust", "Applis de confiance", &name.clone(), move |o| {
            let name = name.clone();
            async move {
                let removed = o.ui_control.untrust(&name);
                o.record("ui_confiance", json!({ "application": name, "action": "retrait" }));
                json!({ "say": if removed { format!("C'est noté : je te demanderai avant de cliquer dans {name}.") } else { format!("{name} n'était pas une appli de confiance.") }, "display": name })
            }
        }));
    }
    if crate::re!(r"^(?:liste|quelles sont|montre|affiche)(?: moi)? (?:les |mes )?(?:applis|applications) de confiance$").is_match(t) {
        return Some(it(o, "ui_trusted_list", "Applis de confiance", "liste", |o| async move {
            let apps = o.ui_control.trusted_apps.borrow().clone();
            let rows: Vec<J> = apps
                .iter()
                .enumerate()
                .map(|(i, a)| json!({ "icon": "✓", "title": format!("{}. {a}", i + 1), "sub": "clics sans accord", "actions": [{ "icon": "✕", "tone": "danger", "title": "Retirer la confiance", "say": format!("Ne fais plus confiance à {a}"), "done": "retirée" }] }))
                .collect();
            o.show_widget(
                "tool",
                json!({
                    "title": "Applis de confiance", "tool": "clics", "count": apps.len(), "ttl": 120000,
                    "empty": "Aucune : je demande ton accord avant chaque clic. Dis « fais confiance à Paint ».", "rows": rows,
                    "footer": "hors de cette liste : accord oral avant chaque clic · jamais dans les gestionnaires de mots de passe, terminaux, registre"
                }),
            );
            json!({ "say": if apps.is_empty() { "Aucune appli de confiance : je te demande avant chaque clic.".to_string() } else { format!("Applis de confiance : {}.", apps.join(", ")) }, "display": format!("{} applis", apps.len()) })
        }));
    }
    if !orch.desktop_state.ready() {
        return None;
    }

    if let Some(m) = crate::re!(r"^(?:quels sont les boutons|quels boutons y a t il|liste les boutons|montre moi les boutons|montre les boutons|liste les elements|qu est ce que je peux cliquer)(?: (?:de|dans|sur|d) (?:l application |l appli |la fenetre )?(.+))?$").captures(t) {
        let spoken = m.get(1).map(|x| x.as_str().to_string()).unwrap_or_default();
        let w = orch.ui_control.target(&spoken);
        let detail = w.as_ref().map(|w| js::gs(w, "app")).unwrap_or_else(|| if spoken.is_empty() { "fenêtre active".into() } else { spoken.clone() });
        return Some(it(o, "ui_list", "Boutons", &detail, move |o| {
            let (w, spoken) = (w.clone(), spoken.clone());
            async move {
                let Some(w) = w else { return json!({ "say": format!("Je ne vois pas {} ouverte.", if spoken.is_empty() { "de fenêtre".into() } else { spoken }), "display": "Introuvable" }) };
                let app = js::gs(&w, "app");
                let oc = o.self_ref();
                let w2 = w.clone();
                let r = o.local_tool("mcp__system__ui_lister", json!({ "application": app }), Box::new(move |_| Box::pin(async move { oc.up().ui_control.list(&w2, 120).await }))).await;
                if !js::gb(&r, "ok") {
                    return json!({ "say": format!("Je n'arrive pas à lire les boutons de {app}."), "display": "Échec" });
                }
                let elements = js::arr(&r["elements"]);
                let els: Vec<&J> = elements.iter().filter(|e| js::gs(e, "type") != "Edit").collect();
                let rows: Vec<J> = elements
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let edit = js::gs(e, "type") == "Edit";
                        let name = js::gs(e, "name");
                        let mut row = json!({ "icon": if edit { "⌨" } else { "▣" }, "title": format!("{}. {name}", i + 1), "sub": format!("{}{}", js::gs(e, "type"), if js::gb(e, "enabled") { "" } else { " · désactivé" }) });
                        if edit {
                            row["hint"] = json!(format!("« remplis {name} avec … dans {app} »"));
                        } else {
                            row["call"] = json!({ "method": "uiClick", "args": [w["hwnd"], i, name] });
                            row["say"] = json!(format!("Clique sur {name} dans {app}"));
                            row["hint"] = json!("Clic : cliquer sur cet élément");
                        }
                        row
                    })
                    .collect();
                o.show_widget(
                    "tool",
                    json!({
                        "title": format!("Éléments de {app}"), "tool": "clics", "wide": true, "count": els.len(), "ttl": 180000,
                        "empty": format!("{app} n’expose pas ses boutons (application sans accessibilité) : dis « clique sur » suivi du texte affiché, je le chercherai à l’écran."),
                        "rows": rows,
                        "footer": format!("« clique sur le bouton 3 » · « clique sur <texte> dans {app} » · {}", if o.ui_control.trusted(Some(&w)) { "appli de confiance : sans accord" } else { "accord oral avant chaque clic" })
                    }),
                );
                let n = els.len();
                let s = if n > 1 { "s" } else { "" };
                json!({ "say": format!("{n} élément{s} cliquable{s} dans {app}. La liste est à l'écran."), "display": format!("{n} éléments") })
            }
        }));
    }

    if let Some(m) = crate::re!(r"^clique(?: moi)? sur (?:le bouton |l element |le numero |le )?(?:numero |n )?(\d{1,3})$").captures(t) {
        let last = orch.ui_control.last_list.borrow().clone();
        if let Some((hwnd, _, elements)) = last {
            let n: usize = m[1].parse().unwrap_or(0);
            let w = orch.desktop_state.by_hwnd(hwnd);
            let el = n.checked_sub(1).and_then(|i| elements.get(i).cloned());
            return Some(it(o, "ui_click_number", "Clic", &format!("n° {n}"), move |o| {
                let (w, el) = (w.clone(), el.clone());
                async move {
                    let (Some(w), Some(el)) = (w, el) else { return json!({ "say": format!("Il n'y a pas d'élément {n}. Dis « quels sont les boutons » pour la liste."), "display": "Introuvable" }) };
                    let name = js::gs(&el, "name");
                    let (w2, n2) = (w.clone(), name.clone());
                    let r = agir(&o, &w, json!({ "action": "cliquer", "cible": name, "numero": n }), Box::new(move |o| Box::pin(async move { o.ui_control.click(&w2, &n2).await }))).await;
                    let said = if js::truthy(&r["name"]) { js::gs(&r, "name") } else { name };
                    ui_outcome(&o, &w, &r, format!("J'ai cliqué sur {said}."))
                }
            }));
        }
    }

    if let Some(m) = crate::re!(r#"(?i)^(?:remplis|renseigne|complète|complete)\s+(?:le champ\s+|la zone\s+)?[«"“]?(.+?)[»"”]?\s+avec\s+[«"“]?(.+?)[»"”]?(?:\s+dans\s+(?:l['’]application\s+|l['’]appli\s+|la fenêtre\s+)?(.+))?$"#).captures(&raw) {
        let (field, value) = (unquote(&m[1]), unquote(&m[2]));
        let app = m.get(3).map(|x| unquote(x.as_str())).unwrap_or_default();
        let w = orch.ui_control.target(&app);
        let detail = format!("{field} ({})", w.as_ref().map(|w| js::gs(w, "app")).unwrap_or_else(|| if app.is_empty() { "fenêtre active".into() } else { app.clone() }));
        return Some(it(o, "ui_fill", "Saisie", &detail, move |o| {
            let (w, field, value, app) = (w.clone(), field.clone(), value.clone(), app.clone());
            async move {
                let Some(w) = w else { return json!({ "say": format!("Je ne vois pas {} ouverte.", if app.is_empty() { "de fenêtre".into() } else { app }), "display": "Introuvable" }) };
                if super::ui_control::UiControl::forbidden(Some(&w)) {
                    return json!({ "say": format!("Pour ta sécurité, je n'écris pas dans {}.", js::gs(&w, "app")), "display": "Refusé" });
                }
                let (w2, f2, v2) = (w.clone(), field.clone(), value.clone());
                let r = agir(&o, &w, json!({ "action": "remplir", "champ": field, "valeur": value }), Box::new(move |o| Box::pin(async move { o.ui_control.fill(&w2, &f2, &v2).await }))).await;
                ui_outcome(&o, &w, &r, format!("C'est écrit dans {field}."))
            }
        }));
    }

    if let Some(m) = crate::re!(r#"(?i)^clique(?:-moi| moi)?\s+sur\s+(?:le bouton\s+|le lien\s+|l['’]onglet\s+|l['’]élément\s+|la case\s+|le menu\s+)?(.+?)(?:\s+(?:dans|de)\s+(?:l['’]application\s+|l['’]appli\s+|la fenêtre\s+)?([^«"“]+))?$"#).captures(&raw) {
        if !crate::re!(r"(?i)^(?:la |l['’])?(?:page|onglet)\s").is_match(&m[1]) {
            let target = unquote(&m[1]);
            let app = m.get(2).map(|x| unquote(x.as_str())).unwrap_or_default();
            let found = orch.ui_control.target(&app);
            let w = if !app.is_empty() && found.is_none() { orch.ui_control.target("") } else { found.clone() };
            let label = if !app.is_empty() && found.is_none() { format!("{target} dans {app}") } else { target.clone() };
            let detail = format!("{label} ({})", w.as_ref().map(|w| js::gs(w, "app")).unwrap_or_else(|| "fenêtre active".into()));
            return Some(it(o, "ui_click", "Clic", &detail, move |o| {
                let (w, label) = (w.clone(), label.clone());
                async move {
                    let Some(w) = w else { return json!({ "say": "Je ne vois aucune fenêtre où cliquer.", "display": "Introuvable" }) };
                    if super::ui_control::UiControl::forbidden(Some(&w)) {
                        return json!({ "say": format!("Pour ta sécurité, je ne clique pas dans {}.", js::gs(&w, "app")), "display": "Refusé" });
                    }
                    // Chrome avec l'extension : clic dans la page affichée de cette instance.
                    if crate::re!(r"(?i)^chrome$").is_match(&js::gs(&w, "process")) && o.browser_bridge.connected() {
                        let list = o.browser_bridge.numbered(Some(&o.desktop_state));
                        let tab = list.iter().find(|x| x["desktop"]["hwnd"] == w["hwnd"]).and_then(|x| js::arr(&x["chrome"]["tabs"]).into_iter().find(|t| js::gb(t, "active")));
                        if let Some(tab) = tab {
                            let oc = o.self_ref();
                            let (tid, l2) = (tab["id"].clone(), label.clone());
                            let r = o
                                .local_tool("mcp__system__onglet_agir", json!({ "action": "cliquer", "page": tab["title"], "texte": label }), Box::new(move |_| Box::pin(async move { oc.up().browser_bridge.request("click", json!({ "tabId": tid, "text": l2 }), 8000).await })))
                                .await;
                            if js::gb(&r, "ok") {
                                return json!({ "say": format!("J'ai cliqué sur {}.", if js::truthy(&r["clicked"]) { js::gs(&r, "clicked") } else { label }), "display": "Clic fait" });
                            }
                        }
                    }
                    let (w2, l2) = (w.clone(), label.clone());
                    let r = agir(&o, &w, json!({ "action": "cliquer", "cible": label }), Box::new(move |o| Box::pin(async move { o.ui_control.click(&w2, &l2).await }))).await;
                    let said = if js::truthy(&r["name"]) { js::gs(&r, "name") } else { label };
                    let ocr = js::gs(&r, "how") == "OCR + clic";
                    ui_outcome(&o, &w, &r, format!("J'ai cliqué sur {said}{}.", if ocr { " (trouvé à l’écran)" } else { "" }))
                }
            }));
        }
    }
    None
}

/// Attend (3 s au plus) que des fenêtres disparaissent ; renvoie celles qui restent.
pub async fn wait_gone(ds: &DesktopState, list: &[J], wait_ms: i64) -> Vec<J> {
    let t0 = js::now_ms();
    let mut remaining = list.to_vec();
    while js::now_ms() - t0 < wait_ms {
        js::sleep_ms(250).await;
        ds.request("snapshot", json!({}), 5000).await;
        remaining = list.iter().filter(|w| ds.by_hwnd(w["hwnd"].as_i64().unwrap_or(0)).is_some()).cloned().collect();
        if remaining.is_empty() {
            break;
        }
    }
    remaining
}
